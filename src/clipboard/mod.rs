pub mod handler;
pub mod monitor;
pub mod storage;

use serde::{Deserialize, Serialize};

/// 剪贴板条目内容类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContentType {
    Text,
    File,
}

impl ContentType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ContentType::Text => "text",
            ContentType::File => "file",
        }
    }

    pub fn parse_content_type(s: &str) -> Self {
        match s {
            "file" => ContentType::File,
            _ => ContentType::Text,
        }
    }
}

/// 从 SQLite 加载的剪贴板条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardItem {
    pub id: i64,
    pub content_type: ContentType,
    pub text_content: Option<String>,
    pub preview: String,
    pub file_paths: Option<Vec<String>>,
    pub content_hash: String,
    pub byte_size: i64,
    pub is_pinned: bool,
    pub source_app: Option<String>,
    pub created_at: String,
}

/// 剪贴板监听到的新内容（待处理）
#[derive(Debug, Clone)]
pub enum RawClipboardContent {
    Text(String),
    Files(Vec<String>),
}

/// 监听线程发送到 app 的事件
#[derive(Debug)]
pub enum ClipboardEvent {
    /// 新增了一条记录 (id)
    ItemAdded(i64),
    /// 记录被删除 (id)
    ItemDeleted(i64),
}
