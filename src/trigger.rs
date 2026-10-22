#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub enum Trigger {
    Command(String),
}

impl Trigger {
    pub fn parse(s: &str) -> Result<Trigger, String> {
        let s = s.trim();
        let (kind, value) = s
            .split_once(':')
            .ok_or_else(|| format!("trigger without \":\" {s:?}"))?;
        let value = value.trim();
        if value.is_empty() {
            return Err(format!("empty trigger {s:?}"));
        }
        match kind.trim().to_ascii_lowercase().as_str() {
            "command" => Ok(Trigger::Command(value.to_string())),
            other     => Err(format!("unknown trigger type: {other:?} in {s:?}")),
        }
    }
}