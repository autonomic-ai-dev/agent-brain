use anyhow::Result;
use std::path::Path;
use tree_sitter::{Language, Parser};

#[derive(Debug)]
pub struct AstSymbol {
    pub symbol_name: String,
    pub symbol_kind: String,
    pub content: String,
    pub file_path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub language: String,
    pub doc_comment: Option<String>,
}

fn language_for_path(path: &Path) -> Option<(Language, &'static str)> {
    let ext = path.extension()?.to_str()?;
    match ext {
        "rs" => Some((tree_sitter_rust::LANGUAGE.into(), "rust")),
        "ts" => Some((tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(), "typescript")),
        "tsx" => Some((tree_sitter_typescript::LANGUAGE_TSX.into(), "typescript")),
        "py" => Some((tree_sitter_python::LANGUAGE.into(), "python")),
        "go" => Some((tree_sitter_go::LANGUAGE.into(), "go")),
        _ => None,
    }
}

pub fn index_file(path: &Path) -> Result<Vec<AstSymbol>> {
    let (lang, lang_name) = match language_for_path(path) {
        Some(l) => l,
        None => return Ok(Vec::new()),
    };

    let source = std::fs::read_to_string(path)?;
    let mut parser = Parser::new();
    parser.set_language(&lang)?;
    let tree = parser
        .parse(&source, None)
        .ok_or_else(|| anyhow::anyhow!("Failed to parse"))?;
    let root = tree.root_node();

    let mut symbols = Vec::new();
    extract_symbols(root, &source, path, lang_name, &mut symbols);
    Ok(symbols)
}

fn extract_symbols(
    node: tree_sitter::Node,
    source: &str,
    path: &Path,
    lang: &str,
    symbols: &mut Vec<AstSymbol>,
) {
    let kind = node.kind();
    let is_definition = matches!(
        kind,
        "function_item"
            | "struct_item"
            | "impl_item"
            | "trait_item"
            | "function_declaration"
            | "class_declaration"
            | "interface_declaration"
            | "function_definition"
            | "class_definition"
            | "method_definition"
    );

    if is_definition {
        let name_node = node.child_by_field_name("name");
        let symbol_name = name_node
            .and_then(|n| source[n.byte_range()].split_whitespace().next())
            .unwrap_or("unnamed")
            .to_string();

        let content = source[node.byte_range()].to_string();
        let doc_comment = extract_doc_comment(node, source);

        symbols.push(AstSymbol {
            symbol_name,
            symbol_kind: kind.to_string(),
            content,
            file_path: path.to_string_lossy().to_string(),
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            language: lang.to_string(),
            doc_comment,
        });
    }

    for child in node.children(&mut node.walk()) {
        extract_symbols(child, source, path, lang, symbols);
    }
}

fn extract_doc_comment(node: tree_sitter::Node, source: &str) -> Option<String> {
    let prev = node.prev_sibling()?;
    let text = source[prev.byte_range()].trim();
    if text.starts_with("///") || text.starts_with("/**") || text.starts_with("//!") {
        Some(text.to_string())
    } else {
        None
    }
}
