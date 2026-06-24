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

#[derive(Debug)]
pub struct AstEdge {
    pub source_symbol: String,
    pub target_symbol: String,
    pub relation: String,
    pub source_file: String,
    pub start_line: usize,
}

fn language_for_path(path: &Path) -> Option<(Language, &'static str)> {
    let ext = path.extension()?.to_str()?;
    match ext {
        "rs" => Some((tree_sitter_rust::LANGUAGE.into(), "rust")),
        "ts" => Some((
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            "typescript",
        )),
        "tsx" => Some((tree_sitter_typescript::LANGUAGE_TSX.into(), "typescript")),
        "py" => Some((tree_sitter_python::LANGUAGE.into(), "python")),
        "go" => Some((tree_sitter_go::LANGUAGE.into(), "go")),
        _ => None,
    }
}

pub fn index_file(path: &Path) -> Result<(Vec<AstSymbol>, Vec<AstEdge>)> {
    let (lang, lang_name) = match language_for_path(path) {
        Some(l) => l,
        None => return Ok((Vec::new(), Vec::new())),
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

    let mut edges = Vec::new();
    extract_edges(root, &source, path, lang_name, &mut edges);

    Ok((symbols, edges))
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

// ── Phase 3: edge extraction ──────────────────────────────

fn extract_edges(
    node: tree_sitter::Node,
    source: &str,
    path: &Path,
    lang: &str,
    edges: &mut Vec<AstEdge>,
) {
    match lang {
        "rust" => extract_rust_edges(node, source, path, edges),
        "typescript" => extract_ts_edges(node, source, path, edges),
        "python" => extract_python_edges(node, source, path, edges),
        "go" => extract_go_edges(node, source, path, edges),
        _ => {}
    }
    for child in node.children(&mut node.walk()) {
        extract_edges(child, source, path, lang, edges);
    }
}

/// Rust: use declarations → "imports", call expressions → "calls", impl items → "implements"
fn extract_rust_edges(
    node: tree_sitter::Node,
    source: &str,
    path: &Path,
    edges: &mut Vec<AstEdge>,
) {
    let file = path.to_string_lossy().to_string();
    match node.kind() {
        "use_declaration" => {
            if let Some(path_node) = child_of_kind(node, "path") {
                let module = source[path_node.byte_range()].trim().to_string();
                edges.push(AstEdge {
                    source_symbol: module.clone(),
                    target_symbol: module,
                    relation: "imports".into(),
                    source_file: file,
                    start_line: node.start_position().row + 1,
                });
            }
        }
        "call_expression" => {
            if let Some(func) = node.child_by_field_name("function") {
                let name = source[func.byte_range()].trim().to_string();
                if !name.starts_with('"') && !name.starts_with('\'') && !name.starts_with('&') {
                    edges.push(AstEdge {
                        source_symbol: name.clone(),
                        target_symbol: name,
                        relation: "calls".into(),
                        source_file: file,
                        start_line: node.start_position().row + 1,
                    });
                }
            }
        }
        "impl_item" => {
            let type_name = node
                .child_by_field_name("type")
                .map(|n| source[n.byte_range()].trim().to_string());
            if let Some(target) = type_name {
                let src = node
                    .child_by_field_name("trait")
                    .map(|n| source[n.byte_range()].trim().to_string())
                    .unwrap_or_default();
                edges.push(AstEdge {
                    source_symbol: src,
                    target_symbol: target,
                    relation: "implements".into(),
                    source_file: file,
                    start_line: node.start_position().row + 1,
                });
            }
        }
        _ => {}
    }
}

/// TypeScript: import statements → "imports", call expressions → "calls", extends/implements → "extends"
fn extract_ts_edges(
    node: tree_sitter::Node,
    source: &str,
    path: &Path,
    edges: &mut Vec<AstEdge>,
) {
    let file = path.to_string_lossy().to_string();
    match node.kind() {
        "import_statement" | "import" => {
            if let Some(src_node) = node.child_by_field_name("source") {
                let module = source[src_node.byte_range()].trim_matches('"').trim_matches('\'').to_string();
                edges.push(AstEdge {
                    source_symbol: module.clone(),
                    target_symbol: module,
                    relation: "imports".into(),
                    source_file: file,
                    start_line: node.start_position().row + 1,
                });
            }
        }
        "call_expression" => {
            if let Some(func) = node.child_by_field_name("function") {
                let name = source[func.byte_range()].trim().to_string();
                if !name.starts_with('"') && !name.starts_with('\'') {
                    edges.push(AstEdge {
                        source_symbol: name.clone(),
                        target_symbol: name,
                        relation: "calls".into(),
                        source_file: file,
                        start_line: node.start_position().row + 1,
                    });
                }
            }
        }
        "class_declaration" => {
            if let Some(base) = node.child_by_field_name("extends") {
                let name = source[base.byte_range()].trim().to_string();
                let src = node
                    .child_by_field_name("name")
                    .map(|n| source[n.byte_range()].trim().to_string())
                    .unwrap_or_default();
                edges.push(AstEdge {
                    source_symbol: src,
                    target_symbol: name,
                    relation: "extends".into(),
                    source_file: file,
                    start_line: node.start_position().row + 1,
                });
            }
        }
        _ => {}
    }
}

/// Python: import/from-import → "imports", calls → "calls", class bases → "extends"
fn extract_python_edges(
    node: tree_sitter::Node,
    source: &str,
    path: &Path,
    edges: &mut Vec<AstEdge>,
) {
    let file = path.to_string_lossy().to_string();
    match node.kind() {
        "import_statement" => {
            for child in node.children(&mut node.walk()) {
                if child.kind() == "dotted_name" {
                    let module = source[child.byte_range()].trim().to_string();
                    edges.push(AstEdge {
                        source_symbol: module.clone(),
                        target_symbol: module,
                        relation: "imports".into(),
                        source_file: file.clone(),
                        start_line: node.start_position().row + 1,
                    });
                }
            }
        }
        "import_from_statement" => {
            let module = child_of_kind(node, "dotted_name")
                .or_else(|| child_of_kind(node, "relative_import"))
                .map(|n| source[n.byte_range()].trim().to_string())
                .unwrap_or_default();
            for child in node.children(&mut node.walk()) {
                if child.kind() == "dotted_name" || child.kind() == "aliased_import" {
                    let name = source[child.byte_range()].trim().to_string();
                    edges.push(AstEdge {
                        source_symbol: name,
                        target_symbol: module.clone(),
                        relation: "imports".into(),
                        source_file: file.clone(),
                        start_line: node.start_position().row + 1,
                    });
                }
            }
        }
        "call" => {
            if let Some(func) = node.child_by_field_name("function") {
                let name = source[func.byte_range()].trim().to_string();
                edges.push(AstEdge {
                    source_symbol: name.clone(),
                    target_symbol: name,
                    relation: "calls".into(),
                    source_file: file.clone(),
                    start_line: node.start_position().row + 1,
                });
            }
        }
        "class_definition" => {
            for child in node.children(&mut node.walk()) {
                if child.kind() == "argument_list" {
                    for arg in child.children(&mut child.walk()) {
                        if arg.kind() == "identifier" || arg.kind() == "attribute" {
                            let base = source[arg.byte_range()].trim().to_string();
                            let src = node
                                .child_by_field_name("name")
                                .map(|n| source[n.byte_range()].trim().to_string())
                                .unwrap_or_default();
                            edges.push(AstEdge {
                                source_symbol: src,
                                target_symbol: base,
                                relation: "extends".into(),
                                source_file: file.clone(),
                                start_line: node.start_position().row + 1,
                            });
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

/// Go: import declarations → "imports", call expressions → "calls"
fn extract_go_edges(
    node: tree_sitter::Node,
    source: &str,
    path: &Path,
    edges: &mut Vec<AstEdge>,
) {
    let file = path.to_string_lossy().to_string();
    match node.kind() {
        "import_declaration" => {
            for child in node.children(&mut node.walk()) {
                if child.kind() == "import_spec" {
                    if let Some(path_node) = child.child_by_field_name("path") {
                        let pkg = source[path_node.byte_range()]
                            .trim_matches('"')
                            .to_string();
                        edges.push(AstEdge {
                            source_symbol: pkg.clone(),
                            target_symbol: pkg,
                            relation: "imports".into(),
                            source_file: file.clone(),
                            start_line: node.start_position().row + 1,
                        });
                    }
                }
            }
        }
        "call_expression" => {
            if let Some(func) = node.child_by_field_name("function") {
                let name = source[func.byte_range()].trim().to_string();
                edges.push(AstEdge {
                    source_symbol: name.clone(),
                    target_symbol: name,
                    relation: "calls".into(),
                    source_file: file.clone(),
                    start_line: node.start_position().row + 1,
                });
            }
        }
        _ => {}
    }
}

fn child_of_kind<'a>(node: tree_sitter::Node<'a>, kind: &str) -> Option<tree_sitter::Node<'a>> {
    node.children(&mut node.walk()).find(|c| c.kind() == kind)
}
