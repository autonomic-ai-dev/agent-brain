use std::path::Path;

use tekmor_core::context_provider::{ContextItem, ContextPayload};
use tekmor_core::types::{FileContext, LessonRec};

use crate::db::store::BrainStore;

pub fn gather(store: &BrainStore, root: &Path, open_files: &[String]) -> Vec<ContextItem> {
    let files = crate::feature_map::route_file_context(root, open_files)
        .into_iter()
        .map(|file| ContextItem {
            payload: ContextPayload::File(file),
        });
    let lessons = crate::lessons::route_lessons(store, root, open_files)
        .into_iter()
        .map(|lesson| ContextItem {
            payload: ContextPayload::Lesson(lesson),
        });
    files.chain(lessons).collect()
}

pub fn split(items: Vec<ContextItem>) -> (Vec<FileContext>, Vec<LessonRec>) {
    let mut files = Vec::new();
    let mut lessons = Vec::new();
    for item in items {
        match item.payload {
            ContextPayload::File(file) => files.push(file),
            ContextPayload::Lesson(lesson) => lessons.push(lesson),
        }
    }
    (files, lessons)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str) -> ContextItem {
        ContextItem {
            payload: ContextPayload::File(FileContext {
                path: path.into(),
                ..FileContext::default()
            }),
        }
    }

    fn lesson(cmd: &str) -> ContextItem {
        ContextItem {
            payload: ContextPayload::Lesson(LessonRec {
                cmd: cmd.into(),
                ..LessonRec::default()
            }),
        }
    }

    #[test]
    fn split_keeps_each_kind_in_arrival_order() {
        let items = vec![file("a.rs"), lesson("first"), file("b.rs"), lesson("second")];
        let (files, lessons) = split(items);
        let paths: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
        let cmds: Vec<_> = lessons.iter().map(|l| l.cmd.as_str()).collect();
        assert_eq!(paths, ["a.rs", "b.rs"]);
        assert_eq!(cmds, ["first", "second"]);
    }

    #[test]
    fn split_of_nothing_is_empty_for_both_fields() {
        let (files, lessons) = split(Vec::new());
        assert!(files.is_empty());
        assert!(lessons.is_empty());
    }
}
