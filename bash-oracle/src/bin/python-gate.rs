//! The Python tree's second and third gates: CPython's own parse of each corpus
//! program, compared node for node with ours, and CPython compiling our print.
//!
//!     cargo run --release -p bash-oracle --bin python-gate -- <corpus.jsonl> [--show <verdict> <n>]
//!
//! The round-trip law cannot see a tree that is consistently wrong and prints
//! back as itself; only a reader that is not ours can. CPython reads the ORIGINAL
//! text — never our print of it, which would only confirm self-consistency.
//!
//! One `python3` for the whole corpus, fed a file of programs with our tree for
//! each, in CPython's own shape ([`reader::syntax::python::canonical()`]).

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;

use reader::shell_ops::Op;
use reader::syntax::python::{canonical, parse, print};

/// Turns CPython's `ast` into the shape `canonical` writes, evaluates our number
/// literals in CPython, and answers one line per program.
const SCRIPT: &str = r#"
import ast, json, sys

SKIP = {"ctx", "type_comment", "type_params", "type_ignores", "returns", "annotation", "kind"}

def theirs(node):
    if isinstance(node, list):
        return [theirs(n) for n in node]
    if node is None or not isinstance(node, ast.AST):
        return node
    if isinstance(node, ast.Constant):
        v = node.value
        if v is True or v is False or v is None:
            return {"_": "Const", "value": repr(v)}
        if v is Ellipsis:
            return {"_": "Const", "value": "Ellipsis"}
        if isinstance(v, str):
            return {"_": "Str", "value": v}
        if isinstance(v, bytes):
            return {"_": "Bytes", "value": list(v)}
        return {"_": "Num", "value": repr(v)}
    if isinstance(node, (ast.operator, ast.unaryop, ast.boolop, ast.cmpop)):
        return type(node).__name__
    out = {"_": type(node).__name__}
    for field, value in ast.iter_fields(node):
        if field not in SKIP:
            out[field] = theirs(value)
    return out

def ours(node):
    if isinstance(node, list):
        return [ours(n) for n in node]
    if isinstance(node, dict):
        if node.get("_") == "Num":
            return {"_": "Num", "value": repr(ast.literal_eval(node["text"]))}
        return {k: ours(v) for k, v in node.items()}
    return node

def first_difference(a, b, path="$"):
    if type(a) is not type(b):
        return path
    if isinstance(a, dict):
        for key in sorted(set(a) | set(b)):
            if key not in a or key not in b:
                return f"{path}.{key}"
            found = first_difference(a[key], b[key], f"{path}.{key}")
            if found:
                return found
        return None
    if isinstance(a, list):
        if len(a) != len(b):
            return f"{path}[len]"
        for i, (x, y) in enumerate(zip(a, b)):
            found = first_difference(x, y, f"{path}[{i}]")
            if found:
                return found
        return None
    return None if a == b else path

for line in open(sys.argv[1]):
    row = json.loads(line)
    answer = {"id": row["id"]}
    try:
        tree = ast.parse(row["source"])
    except (SyntaxError, ValueError) as e:
        answer["verdict"] = "cpython refuses" if row["ours"] is not None else "both refuse"
        print(json.dumps(answer)); continue
    if row["ours"] is None:
        answer["verdict"] = "we refuse"
        print(json.dumps(answer)); continue
    a, b = ours(row["ours"]), theirs(tree)
    if a != b:
        answer["verdict"] = "trees differ"
        answer["path"] = first_difference(a, b)
        print(json.dumps(answer)); continue
    try:
        compile(row["printed"], "<print>", "exec")
        answer["verdict"] = "agree"
    except SyntaxError:
        answer["verdict"] = "print does not compile"
    print(json.dumps(answer))
"#;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1) else {
        anyhow::bail!("usage: python-gate <corpus.jsonl> [--show <verdict> <n>]");
    };
    let show = args.iter().position(|a| a == "--show").and_then(|at| {
        Some((
            args.get(at + 1)?.clone(),
            args.get(at + 2)?.parse::<usize>().ok()?,
        ))
    });
    let home = std::env::var("HOME").unwrap_or_default();

    let mut programs = BTreeSet::new();
    for line in std::fs::read_to_string(path)?.lines() {
        let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(cmd) = row["cmd"].as_str() else {
            continue;
        };
        let Ok(parsed) = reader::project::read(cmd) else {
            continue;
        };
        let found = reader::shell_files::extract_knowing(&parsed, row["cwd"].as_str(), &home, &[]);
        for op in found.ops {
            if let Op::Python { source } = op {
                programs.insert(source);
            }
        }
    }
    let programs: Vec<String> = programs.into_iter().collect();

    let input = std::env::temp_dir().join(format!("python-gate-{}.jsonl", std::process::id()));
    {
        let mut out = std::io::BufWriter::new(std::fs::File::create(&input)?);
        for (id, source) in programs.iter().enumerate() {
            let tree = parse(source).ok();
            let row = serde_json::json!({
                "id": id,
                "source": source,
                "ours": tree.as_ref().map(canonical),
                "printed": tree.as_ref().map(print),
            });
            writeln!(out, "{row}")?;
        }
    }
    let ran = std::process::Command::new("python3")
        .args(["-W", "ignore", "-c", SCRIPT])
        .arg(&input)
        .output()?;
    let _ = std::fs::remove_file(&input);
    if !ran.status.success() {
        anyhow::bail!("python3 failed: {}", String::from_utf8_lossy(&ran.stderr));
    }
    let version = std::process::Command::new("python3")
        .arg("--version")
        .output()?;

    let mut verdicts: BTreeMap<String, usize> = BTreeMap::new();
    let mut paths: BTreeMap<String, usize> = BTreeMap::new();
    let mut shown = 0usize;
    for line in String::from_utf8_lossy(&ran.stdout).lines() {
        let answer: serde_json::Value = serde_json::from_str(line)?;
        let verdict = answer["verdict"].as_str().unwrap_or("?").to_string();
        *verdicts.entry(verdict.clone()).or_insert(0) += 1;
        if let Some(path) = answer["path"].as_str() {
            // The node kinds along the path, not the indices: what to look at.
            let shape: String = path
                .split('[')
                .map(|p| p.rsplit(']').next().unwrap_or(""))
                .collect();
            *paths.entry(shape).or_insert(0) += 1;
        }
        if let Some((wanted, n)) = &show
            && verdict.contains(wanted.as_str())
            && shown < *n
        {
            shown += 1;
            let id = answer["id"].as_u64().unwrap_or(0) as usize;
            println!("--- {verdict} {}:\n{}\n", answer["path"], programs[id]);
        }
    }

    println!("{}", String::from_utf8_lossy(&version.stdout).trim());
    println!("python programs        {}", programs.len());
    for (verdict, n) in &verdicts {
        println!("  {verdict:<24} {n}");
    }
    if !paths.is_empty() {
        println!("where the trees first differ:");
        let mut ranked: Vec<_> = paths.into_iter().collect();
        ranked.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        for (path, n) in ranked.iter().take(20) {
            println!("  {n:6}  {path}");
        }
    }
    Ok(())
}
