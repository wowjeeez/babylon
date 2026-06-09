use serde::{Deserialize, Deserializer};

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default = "default_db")]
    pub db_path: String,
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default, deserialize_with = "de_lenient_bool")]
    pub dev_no_auth: bool,
    #[serde(default, deserialize_with = "de_lenient_bool")]
    pub allow_funnel: bool,
}

fn default_db() -> String {
    "babylon.db".into()
}

fn default_bind() -> String {
    "127.0.0.1:8787".into()
}

fn de_lenient_bool<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Bool(bool),
        Int(i64),
        Str(String),
    }
    match Raw::deserialize(deserializer)? {
        Raw::Bool(b) => Ok(b),
        Raw::Int(n) => Ok(n != 0),
        Raw::Str(s) => match s.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" | "" => Ok(false),
            other => Err(D::Error::custom(format!("invalid boolean: {other}"))),
        },
    }
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        use figment::providers::{Env, Format, Toml};
        Ok(figment::Figment::new()
            .merge(Toml::file("babylon.toml"))
            .merge(Env::prefixed("BABYLON_"))
            .extract()?)
    }
}
