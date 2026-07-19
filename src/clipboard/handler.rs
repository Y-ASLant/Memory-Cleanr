use anyhow::Result;

use super::RawClipboardContent;

const MAX_PREVIEW_LEN: usize = 200;
const MAX_TEXT_SIZE: usize = 1_048_576; // 1 MB

/// Processed clipboard content ready for storage.
pub struct ProcessedContent {
    pub content_type: super::ContentType,
    pub text_content: Option<String>,
    pub preview: String,
    pub file_paths: Option<Vec<String>>,
    pub content_hash: String,
    pub byte_size: i64,
}

/// Process raw clipboard content: truncate, preview, hash.
pub fn process(raw: RawClipboardContent, source_app: Option<&str>) -> Result<ProcessedContent> {
    match raw {
        RawClipboardContent::Text(text) => process_text(text),
        RawClipboardContent::Files(paths) => process_files(paths, source_app),
    }
}

fn process_text(text: String) -> Result<ProcessedContent> {
    // Truncate oversized text
    let text = if text.len() > MAX_TEXT_SIZE {
        // Safe truncation at char boundary
        let mut end = MAX_TEXT_SIZE;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        text[..end].to_string()
    } else {
        text
    };

    let preview = truncate_chars(&text, MAX_PREVIEW_LEN);
    let content_hash = hash_text(&text);
    let byte_size = text.len() as i64;

    Ok(ProcessedContent {
        content_type: super::ContentType::Text,
        text_content: Some(text),
        preview,
        file_paths: None,
        content_hash,
        byte_size,
    })
}

fn process_files(paths: Vec<String>, _source_app: Option<&str>) -> Result<ProcessedContent> {
    let preview = if paths.len() == 1 {
        std::path::Path::new(&paths[0])
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| paths[0].clone())
    } else {
        let first = std::path::Path::new(&paths[0])
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| paths[0].clone());
        format!("{} (+{})", first, paths.len() - 1)
    };

    // Hash the sorted file paths
    let mut sorted = paths.clone();
    sorted.sort();
    let content_hash = hash_text(&sorted.join("\0"));

    let byte_size: i64 = paths
        .iter()
        .map(|p| std::fs::metadata(p).map(|m| m.len() as i64).unwrap_or(0))
        .sum();

    Ok(ProcessedContent {
        content_type: super::ContentType::File,
        text_content: None,
        preview,
        file_paths: Some(paths),
        content_hash,
        byte_size,
    })
}

/// Compute BLAKE3 hash of text content.
pub fn hash_text(text: &str) -> String {
    let hash = blake3::hash(text.as_bytes());
    hash.to_hex().to_string()
}

/// Truncate string to `max_chars` Unicode characters, appending "…" if truncated.
fn truncate_chars(s: &str, max_chars: usize) -> String {
    let mut byte_end = s.len();
    for (count, (i, _)) in s.char_indices().enumerate() {
        if count >= max_chars {
            byte_end = i;
            break;
        }
    }
    if byte_end < s.len() {
        format!("{}…", &s[..byte_end])
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_preview_truncation() {
        let long = "a".repeat(500);
        let result = process_text(long).unwrap();
        assert!(result.preview.len() <= 210); // 200 chars + "…"
        assert!(result.preview.ends_with('…'));
    }

    #[test]
    fn text_hash_deterministic() {
        assert_eq!(hash_text("hello"), hash_text("hello"));
        assert_ne!(hash_text("hello"), hash_text("world"));
    }

    #[test]
    fn file_single_preview() {
        let result = process_files(vec!["C:\\test\\file.txt".into()], None).unwrap();
        assert_eq!(result.preview, "file.txt");
    }

    #[test]
    fn file_multi_preview() {
        let result = process_files(
            vec!["C:\\a.txt".into(), "C:\\b.txt".into(), "C:\\c.txt".into()],
            None,
        )
        .unwrap();
        assert_eq!(result.preview, "a.txt (+2)");
    }

    #[test]
    fn truncate_short_string() {
        assert_eq!(truncate_chars("abc", 10), "abc");
    }

    #[test]
    fn truncate_exact_boundary() {
        assert_eq!(truncate_chars("abcde", 5), "abcde");
    }

    #[test]
    fn truncate_utf8_boundary() {
        // "你好世界" = 4 chars; max_chars=2 keeps 2 full chars + "…"
        assert_eq!(truncate_chars("你好世界", 2), "你好…");
    }
}
