//! Gateway settings shared by startup, enforcement, and response metadata.
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayLimits {
    pub per_caller: usize,
    pub system_wide: usize,
    pub window: Duration,
    pub body_bytes: usize,
}

impl Default for GatewayLimits {
    fn default() -> Self {
        Self {
            per_caller: 100,
            system_wide: 1000,
            window: Duration::from_secs(60),
            body_bytes: crate::server::DEFAULT_BODY_LIMIT,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("invalid gateway limit: {0}")]
pub struct InvalidGatewayLimits(pub String);

impl GatewayLimits {
    pub fn validate(&self) -> Result<(), InvalidGatewayLimits> {
        if self.per_caller == 0
            || self.system_wide == 0
            || self.window.is_zero()
            || self.body_bytes == 0
        {
            return Err(InvalidGatewayLimits(
                "all allowances, the window, and the body bound must be positive".into(),
            ));
        }
        Ok(())
    }

    /// Read explicit operator overrides. Invalid values fail startup instead
    /// of silently disabling a protection or falling back to another value.
    pub fn from_env() -> Result<Self, InvalidGatewayLimits> {
        Self::read(|name| match std::env::var(name) {
            Ok(value) => Ok(Some(value)),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(error) => Err(InvalidGatewayLimits(format!("{name}: {error}"))),
        })
    }

    fn read(
        mut value: impl FnMut(&str) -> Result<Option<String>, InvalidGatewayLimits>,
    ) -> Result<Self, InvalidGatewayLimits> {
        let mut limits = Self::default();
        for (name, setting) in [
            ("MALT_GATEWAY_REQUESTS_PER_WINDOW", &mut limits.per_caller),
            (
                "MALT_GATEWAY_GLOBAL_REQUESTS_PER_WINDOW",
                &mut limits.system_wide,
            ),
            ("MALT_GATEWAY_BODY_LIMIT_BYTES", &mut limits.body_bytes),
        ] {
            if let Some(raw) = value(name)? {
                *setting = raw.parse().map_err(|_| {
                    InvalidGatewayLimits(format!("{name} must be a positive integer"))
                })?;
            }
        }
        if let Some(raw) = value("MALT_GATEWAY_WINDOW_SECS")? {
            let seconds = raw.parse().map_err(|_| {
                InvalidGatewayLimits("MALT_GATEWAY_WINDOW_SECS must be a positive integer".into())
            })?;
            limits.window = Duration::from_secs(seconds);
        }
        limits.validate()?;
        Ok(limits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_are_validated_without_mutating_process_environment() {
        let limits = GatewayLimits::read(|key| {
            Ok(match key {
                "MALT_GATEWAY_REQUESTS_PER_WINDOW" => Some("7".into()),
                "MALT_GATEWAY_GLOBAL_REQUESTS_PER_WINDOW" => Some("20".into()),
                "MALT_GATEWAY_WINDOW_SECS" => Some("2".into()),
                "MALT_GATEWAY_BODY_LIMIT_BYTES" => Some("4096".into()),
                _ => None,
            })
        })
        .unwrap();
        assert_eq!(
            limits,
            GatewayLimits {
                per_caller: 7,
                system_wide: 20,
                window: Duration::from_secs(2),
                body_bytes: 4096
            }
        );
        for raw in ["0", "", "-1", "NaN", "184467440737095516160"] {
            assert!(GatewayLimits::read(|_| Ok(Some(raw.into()))).is_err());
        }
        assert_eq!(
            GatewayLimits::read(|_| Ok(None)).unwrap(),
            GatewayLimits::default()
        );
    }
}
