param([string]$Executable = (Join-Path $PSScriptRoot '../target/debug/malt.exe'))

# Real CLI-daemon proof. Owns only the child started here and its isolated
# session store. The developer's running daemon and helper are untouched.
$ErrorActionPreference = 'Stop'
$Executable = (Resolve-Path -LiteralPath $Executable).Path
$fixtureRoot = Join-Path ([IO.Path]::GetTempPath()) ('malt-gateway-proof-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $fixtureRoot | Out-Null
$listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
$listener.Start()
$port = $listener.LocalEndpoint.Port
$listener.Stop()
$settings = [Diagnostics.ProcessStartInfo]::new($Executable)
$settings.UseShellExecute = $false
$settings.CreateNoWindow = $true
$settings.RedirectStandardOutput = $true
$settings.RedirectStandardError = $true
$settings.ArgumentList.Add('daemon')
$settings.ArgumentList.Add('--port')
$settings.ArgumentList.Add([string]$port)
$settings.Environment['LOCALAPPDATA'] = $fixtureRoot
$settings.Environment['XDG_DATA_HOME'] = $fixtureRoot
$settings.Environment['MALT_GATEWAY_REQUESTS_PER_WINDOW'] = '3'
$settings.Environment['MALT_GATEWAY_GLOBAL_REQUESTS_PER_WINDOW'] = '12'
$settings.Environment['MALT_GATEWAY_WINDOW_SECS'] = '2'
$settings.Environment['MALT_GATEWAY_BODY_LIMIT_BYTES'] = '2097152'
$fixtureDaemon = [Diagnostics.Process]::Start($settings)
$stdout = $fixtureDaemon.StandardOutput.ReadToEndAsync()
$stderr = $fixtureDaemon.StandardError.ReadToEndAsync()
$handler = [Net.Http.HttpClientHandler]::new()
$handler.UseProxy = $false
$client = [Net.Http.HttpClient]::new($handler)
$client.Timeout = [TimeSpan]::FromSeconds(15)
$base = "http://127.0.0.1:$port"
$payloadPath = $null

function Invoke-ProofRequest([string]$Method, [string]$Path, [byte[]]$Body) {
    $request = [Net.Http.HttpRequestMessage]::new([Net.Http.HttpMethod]::new($Method), "$base$Path")
    $request.Headers.Authorization = [Net.Http.Headers.AuthenticationHeaderValue]::new('Bearer', $script:proofToken)
    if ($null -ne $Body) {
        $request.Content = [Net.Http.ByteArrayContent]::new($Body)
        $request.Content.Headers.ContentType = [Net.Http.Headers.MediaTypeHeaderValue]::new('application/json')
    }
    try {
        $response = $client.Send($request)
        try {
            [pscustomobject]@{ Status = [int]$response.StatusCode; Headers = $response.Headers; Text = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult() }
        } finally { $response.Dispose() }
    } finally { $request.Dispose() }
}

function Assert-Status($Response, [int]$Expected) {
    if ($Response.Status -ne $Expected) { throw "Expected $Expected, received $($Response.Status): $($Response.Text)" }
}

try {
    $ready = $false
    for ($attempt = 0; $attempt -lt 100; $attempt++) {
        if ($fixtureDaemon.HasExited) { throw "Daemon exited: $($stderr.GetAwaiter().GetResult())" }
        $socket = [Net.Sockets.TcpClient]::new()
        try { $socket.Connect('127.0.0.1', $port); $ready = $true; break } catch { Start-Sleep -Milliseconds 50 } finally { $socket.Dispose() }
    }
    if (!$ready) { throw 'Daemon never bound its HTTP listener' }
    $tokenRoot = if ($env:HOME) { $env:HOME } else { $env:USERPROFILE }
    $script:proofToken = [IO.File]::ReadAllText((Join-Path $tokenRoot '.config/malt/api-token')).Trim()
    $remaining = @()
    for ($index = 0; $index -lt 3; $index++) {
        $response = Invoke-ProofRequest GET '/health' $null
        Assert-Status $response 200
        $remaining += [int]($response.Headers.GetValues('X-RateLimit-Remaining') | Select-Object -First 1)
    }
    if (($remaining -join ',') -ne '2,1,0') { throw "Unexpected allowances: $remaining" }
    $refused = Invoke-ProofRequest GET '/health' $null
    Assert-Status $refused 429
    if (($refused.Text | ConvertFrom-Json).error.cause -ne 'per_caller') { throw 'Missing refusal cause' }
    $retry = [int]($refused.Headers.GetValues('Retry-After') | Select-Object -First 1)
    Start-Sleep -Seconds $retry
    Assert-Status (Invoke-ProofRequest GET '/health' $null) 200
    Start-Sleep -Seconds 2

    $create = Invoke-ProofRequest POST '/sessions' ([Text.Encoding]::UTF8.GetBytes('{"name":"gateway-live-proof","isolation":"bare"}'))
    Assert-Status $create 200
    $session = ($create.Text | ConvertFrom-Json).data.id
    $command = 'echo gateway-live-ok #' + ('x' * 65536)
    $json = ConvertTo-Json -Compress @{ command = $command }
    $bytes = [Text.Encoding]::UTF8.GetBytes($json)
    $execBytes = $bytes.Length
    $boundary = [byte[]]::new(2097152)
    [Array]::Fill[byte]($boundary, 32)
    [Array]::Copy($bytes, $boundary, $bytes.Length)
    $exec = Invoke-ProofRequest POST "/sessions/$session/exec" $boundary
    Assert-Status $exec 200
    if (($exec.Text | ConvertFrom-Json).data.output -notmatch 'gateway-live-ok') { throw 'Real MASH output did not round-trip' }
    $send = [Text.Encoding]::UTF8.GetBytes((ConvertTo-Json -Compress @{ input = ('x' * 8192) }))
    Assert-Status (Invoke-ProofRequest POST "/sessions/$session/send" $send) 200
    Start-Sleep -Seconds 2

    # Provide a 128 MiB content stream. Expect/Continue lets the server refuse
    # the declared length before uploading it. Child RSS is sampled separately.
    $payloadPath = Join-Path $fixtureRoot 'oversize-body.bin'
    $payload = [IO.File]::Open($payloadPath, [IO.FileMode]::CreateNew, [IO.FileAccess]::ReadWrite)
    $payload.SetLength(134217728)
    $fixtureDaemon.Refresh()
    $before = $fixtureDaemon.WorkingSet64
    $large = [Net.Http.HttpRequestMessage]::new([Net.Http.HttpMethod]::Post, "$base/sessions/$session/exec")
    $large.Headers.Authorization = [Net.Http.Headers.AuthenticationHeaderValue]::new('Bearer', $script:proofToken)
    $large.Headers.ExpectContinue = $true
    $large.Content = [Net.Http.StreamContent]::new($payload)
    $large.Content.Headers.ContentType = [Net.Http.Headers.MediaTypeHeaderValue]::new('application/json')
    $peak = $before
    try {
        $pending = $client.SendAsync($large)
        while (!$pending.IsCompleted) {
            $fixtureDaemon.Refresh()
            $peak = [Math]::Max($peak, $fixtureDaemon.WorkingSet64)
            Start-Sleep -Milliseconds 2
        }
        $response = $pending.GetAwaiter().GetResult()
        try {
            if ([int]$response.StatusCode -ne 413) { throw "Oversize status: $($response.StatusCode)" }
            $detail = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult() | ConvertFrom-Json
            if ($detail.error.code -ne 'payload_too_large') { throw 'Missing structured size refusal' }
        } finally { $response.Dispose() }
    } finally { $large.Dispose(); $payload.Dispose() }
    $fixtureDaemon.Refresh()
    $peak = [Math]::Max($peak, $fixtureDaemon.WorkingSet64)
    $delta = $peak - $before
    if ($delta -gt 33554432) { throw "Oversized request raised daemon RSS by $delta bytes" }
    Start-Sleep -Seconds 2
    Assert-Status (Invoke-ProofRequest DELETE "/sessions/$session" $null) 200
    Assert-Status (Invoke-ProofRequest POST '/shutdown' $null) 200
    if (!$fixtureDaemon.WaitForExit(10000)) { throw 'Proof daemon did not shut down' }
    if ($fixtureDaemon.ExitCode -ne 0) { throw "Daemon exit code $($fixtureDaemon.ExitCode)" }
    [pscustomobject]@{ recovery = 'passed'; retry_after_secs = $retry; remaining = ($remaining -join ','); real_mash_boundary_bytes = $boundary.Length; exec_json_bytes = $execBytes; send_json_bytes = $send.Length; oversize_bytes = 134217728; peak_daemon_rss_delta = $delta; shutdown_exit = $fixtureDaemon.ExitCode; evidence_directory = $fixtureRoot } | ConvertTo-Json
} finally {
    if (!$fixtureDaemon.HasExited) { $fixtureDaemon.Kill($true); $fixtureDaemon.WaitForExit() }
    $client.Dispose()
    $fixtureDaemon.Dispose()
    $script:proofToken = $null
    if ($payloadPath -and (Test-Path -LiteralPath $payloadPath)) {
        Remove-Item -LiteralPath $payloadPath
    }
}
