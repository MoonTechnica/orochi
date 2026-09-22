use crate::{
    context::{changed_files, git},
    types::{Complexity, TaskDescriptor},
};
use std::{collections::BTreeMap, path::Path};

/// Smallest scope a complexity implies, with no candidate files to go on.
pub fn scope_floor(complexity: Complexity) -> usize {
    match complexity {
        Complexity::Simple => 1,
        Complexity::Normal => 4,
        Complexity::Complex => 12,
        Complexity::Extreme => 30,
    }
}

pub fn profile(task: &str, root: &Path) -> TaskDescriptor {
    let lower = task.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| lower.contains(w));
    // Supporting deliverables (for example "also write a README and run tests")
    // must not turn an implementation request into a cheap documentation task.
    // Use the opening request for these narrower categories and conservatively
    // retain implementation when it also explicitly asks to build software.
    let opening = lower
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    let opening_has = |words: &[&str]| words.iter().any(|w| opening.contains(w));
    let builds_software = opening.trim_start().starts_with("implement ")
        || opening_has(&[
            "build an app",
            "build a web",
            "create an app",
            "create a web",
            "implement an app",
            "implement a web",
            "implement a feature",
            "機能を実装",
            "機能を追加",
        ])
        || (opening_has(&["アプリ"]) && opening_has(&["実装", "作成", "作って", "開発"]));
    // Asking for the agents themselves to work together is a request for more than one seat.
    let together = has(&[
        "会話",
        "話し合",
        "ディスカッション",
        "議論",
        "相談",
        "協力",
        "雑談",
        "ブレスト",
        "座談",
        "discussion",
        "discuss",
        "brainstorm",
        "talk to each other",
        "talk with each other",
        "collaborate",
        "debate",
    ]);
    let collaborative = has(&[
        "エージェント同士",
        "エージェントどうし",
        "エージェント間",
        "agent to agent",
        "agents talk",
        "between agents",
        "with another agent",
    ]) || (has(&["エージェント", "agent", "モデル"]) && together);
    // Talking it through is the whole request when nothing is asked to be built.
    let builds = has(&[
        "実装",
        "修正",
        "直し",
        "作っ",
        "書いて",
        "リファクタ",
        "移行",
        "追加",
        "implement",
        "fix",
        "build",
        "write",
        "refactor",
        "migrate",
        "add ",
    ]);
    let task_type = if collaborative && !builds {
        "discussion"
    } else if has(&["migration", "migrate", "移行"]) {
        "migration"
    } else if has(&["architecture", "アーキテクチャ", "設計変更"]) {
        "architecture"
    } else if has(&["refactor", "リファクタ"]) {
        "refactor"
    } else if has(&["review", "レビュー"]) {
        "review"
    } else if has(&["bug", "fix", "不具合", "バグ", "修正"]) {
        "bug_fix"
    } else if !builds_software
        && opening_has(&["readme", "documentation", "ドキュメント", "説明文"])
    {
        "documentation"
    } else if !builds_software && opening_has(&["test", "テスト"]) {
        "test"
    } else if has(&["investigate", "調査", "原因"]) {
        "investigation"
    } else if has(&["typo", "rename", "誤字", "名称変更"]) {
        "small_edit"
    } else {
        "implementation"
    };
    let files: Vec<String> = if let Some(value) = git(
        root,
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ],
    ) {
        value
            .split('\0')
            .filter(|s| !s.is_empty())
            .take(20_000)
            .map(String::from)
            .collect()
    } else {
        walkdir::WalkDir::new(root)
            .follow_links(false)
            .max_depth(12)
            .into_iter()
            .filter_entry(|e| {
                !e.file_type().is_dir()
                    || !matches!(
                        e.file_name().to_str(),
                        Some(
                            ".git"
                                | "node_modules"
                                | "target"
                                | "vendor"
                                | "dist"
                                | ".venv"
                                | ".orochi"
                        )
                    )
            })
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
            .take(20_000)
            .filter_map(|e| {
                e.path()
                    .strip_prefix(root)
                    .ok()
                    .map(|p| p.to_string_lossy().into_owned())
            })
            .collect()
    };
    let mut counts = BTreeMap::<&str, usize>::new();
    for file in &files {
        let language = match Path::new(file).extension().and_then(|x| x.to_str()) {
            Some("rs") => "rust",
            Some("ts" | "tsx") => "typescript",
            Some("js" | "jsx" | "mjs") => "javascript",
            Some("py") => "python",
            Some("go") => "go",
            Some("rb") => "ruby",
            Some("java" | "kt") => "jvm",
            Some("swift") => "swift",
            _ => continue,
        };
        *counts.entry(language).or_default() += 1;
    }
    let language = counts
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map(|(l, _)| l)
        .unwrap_or("unknown")
        .to_string();
    let package: serde_json::Value = std::fs::read(root.join("package.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let framework = ["next", "react", "vue", "svelte", "express"]
        .iter()
        .find(|name| {
            package["dependencies"][**name].is_object()
                || package["dependencies"][**name].is_string()
                || package["devDependencies"][**name].is_string()
        })
        .map(|s| s.to_string());
    let mut candidates: Vec<String> = files
        .iter()
        .filter(|f| {
            lower.contains(&f.to_lowercase())
                || Path::new(f)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.len() > 4 && lower.contains(&n.to_lowercase()))
        })
        .cloned()
        .collect();
    candidates.extend(changed_files(root));
    candidates.sort();
    candidates.dedup();
    candidates.truncate(30);
    let architecture = matches!(task_type, "architecture" | "migration")
        || has(&["across the", "全体", "全面", "刷新"]);
    let long_horizon = lower
        .split(|c: char| !c.is_ascii_alphabetic())
        .any(|word| word == "entire")
        || has(&["end-to-end", "from scratch", "全面", "新規構築", "大規模"])
        || task.chars().count() > 3000;
    let simple =
        matches!(task_type, "small_edit" | "documentation") && !architecture && !long_horizon;
    let complexity = if architecture && long_horizon {
        Complexity::Extreme
    } else if architecture || long_horizon || task_type == "refactor" {
        Complexity::Complex
    } else if simple {
        Complexity::Simple
    } else {
        Complexity::Normal
    };
    let scope = candidates.len().max(scope_floor(complexity));
    let tests_available = root.join("Cargo.toml").exists()
        || root.join("go.mod").exists()
        || package["scripts"]["test"].is_string()
        || files
            .iter()
            .any(|f| f.contains("test_") || f.contains(".test.") || f.contains("/tests/"));
    TaskDescriptor {
        task_type: task_type.into(),
        language,
        framework,
        repo_size: files.len(),
        candidate_files: candidates,
        estimated_scope: scope,
        estimated_context: (scope as u64 * 1500).min(120_000) + task.len() as u64 / 3,
        complexity,
        requires_architecture_change: architecture,
        requires_browser: has(&[
            "browser",
            "ブラウザ",
            "playwright",
            "screenshot",
            "スクリーンショット",
        ]),
        requires_web: has(&["web search", "search the web", "ウェブ検索", "ネットで調べ"]),
        requires_image: has(&[
            "generate an image",
            "generate images",
            "create an image",
            "draw an image",
            "画像を生成",
            "画像生成",
            "イラストを作",
            "画像を作",
            "図を生成",
        ]),
        collaborative,
        seats: seats_asked_for(task),
        tests_available,
        ambiguity: if task.chars().count() < 12 {
            0.9
        } else if task.chars().count() < 40 {
            0.55
        } else {
            0.2
        },
        long_horizon,
        preferred: None,
        suited: None,
    }
}

/// How many agents the user asked for ("5人くらいのエージェントで…", "with 3 agents").
fn seats_asked_for(task: &str) -> Option<usize> {
    let chars: Vec<char> = task.chars().collect();
    let mut found = None;
    for (index, c) in chars.iter().enumerate() {
        if !c.is_ascii_digit()
            || chars
                .get(index.wrapping_sub(1))
                .is_some_and(char::is_ascii_digit)
        {
            continue;
        }
        let digits: String = chars[index..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        let rest: String = chars[index + digits.len()..].iter().take(12).collect();
        let counts = ["人", "体", "名", "つ", "個"]
            .iter()
            .any(|unit| rest.starts_with(unit))
            || rest.trim_start().starts_with("agent");
        if counts && let Ok(count) = digits.parse::<usize>() {
            found = Some(found.map_or(count, |seen: usize| seen.max(count)));
        }
    }
    found
}
