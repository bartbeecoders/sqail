use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum AiProviderType {
    Claude,
    OpenAi,
    OpenAiCompatible,
    Minimax,
    Zai,
    ClaudeCodeCli,
    LmStudio,
}

impl std::fmt::Display for AiProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AiProviderType::Claude => write!(f, "claude"),
            AiProviderType::OpenAi => write!(f, "openAi"),
            AiProviderType::OpenAiCompatible => write!(f, "openAiCompatible"),
            AiProviderType::Minimax => write!(f, "minimax"),
            AiProviderType::Zai => write!(f, "zai"),
            AiProviderType::ClaudeCodeCli => write!(f, "claudeCodeCli"),
            AiProviderType::LmStudio => write!(f, "lmStudio"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderConfig {
    pub id: String,
    pub name: String,
    pub provider: AiProviderType,
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiHistoryEntry {
    pub id: String,
    pub timestamp: String,
    pub flow: String,
    pub prompt: String,
    pub response: String,
    #[serde(default)]
    pub connection_id: Option<String>,
}
