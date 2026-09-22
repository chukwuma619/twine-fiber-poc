use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofMeta {
    pub content_type: String,
    pub bytes: usize,
}
