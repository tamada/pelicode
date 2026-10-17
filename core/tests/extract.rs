use pelicode_core::{Config, Language, Level, extract_source};

fn config() -> Config {
    Config {
        min_tokens: 0,
        keep_tokens: true,
        ..Config::default()
    }
}

fn single(language: Language, source: &str) -> pelicode_core::FunctionFeatures {
    let functions = extract_source(language, source.as_bytes(), &config()).unwrap();
    assert_eq!(functions.len(), 1, "{functions:#?}");
    functions.into_iter().next().unwrap()
}

fn assert_same(language: Language, a: &str, b: &str, level: Level) {
    let fa = single(language, a);
    let fb = single(language, b);
    assert_eq!(
        fa.exact_hashes[&level],
        fb.exact_hashes[&level],
        "{level}: {:?} vs {:?}",
        fa.tokens.as_ref().unwrap()[&level],
        fb.tokens.as_ref().unwrap()[&level]
    );
}

fn assert_differ(language: Language, a: &str, b: &str, level: Level) {
    let fa = single(language, a);
    let fb = single(language, b);
    assert_ne!(fa.exact_hashes[&level], fb.exact_hashes[&level], "{level}");
}

const JAVA_A: &str = r#"
class A {
    int sum(int[] values) {
        // add everything
        int total = 0;
        for (int v : values) { total += v * 2; }
        return total;
    }
}
"#;

const JAVA_B: &str = r#"
class B {
    int add(int[] xs) {
        int acc = 0;
        for (int x : xs) {
            acc += x * 3;
        }
        return acc;
    }
}
"#;

#[test]
fn java_renaming_matches_from_n1() {
    assert_differ(Language::Java, JAVA_A, JAVA_B, Level::N0);
    assert_same(Language::Java, JAVA_A, JAVA_B, Level::N1);
    assert_same(Language::Java, JAVA_A, JAVA_B, Level::N2);
    let f = single(Language::Java, JAVA_A);
    assert_eq!(f.name, "sum");
    assert_eq!((f.start_line, f.end_line), (3, 8));
}

#[test]
fn java_types_are_abstracted_at_n2() {
    let a = "class A { List<String> f(Map<String, Integer> m) { return null; } }";
    let b = "class A { Set<Long> f(Tree<Long, Double> m) { return null; } }";
    assert_differ(Language::Java, a, b, Level::N1);
    assert_same(Language::Java, a, b, Level::N2);
}

#[test]
fn python_ignores_comments_and_docstrings() {
    let a = r#"
def area(w, h):
    """Return the area."""
    # multiply
    return w * h
"#;
    let b = "def area(w, h):\n    return w*h\n";
    assert_same(Language::Python, a, b, Level::N0);
}

#[test]
fn nested_functions_are_extracted_separately() {
    let src = r#"
def outer(xs):
    def inner(x):
        return x + 1
    return [inner(x) for x in xs]
"#;
    let functions = extract_source(Language::Python, src.as_bytes(), &config()).unwrap();
    let names: Vec<_> = functions.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["outer", "inner"]);
    assert_eq!(functions[1].parent, Some(0));
}

#[test]
fn go_methods_and_literals() {
    let a = r#"
package main
func (s *Stack) Push(v int) { s.items = append(s.items, v); fmt.Println("pushed") }
"#;
    let b = r#"
package main
func (q *Stack) Add(x int) {
	q.data = append(q.data, x)
	fmt.Println("added")
}
"#;
    assert_same(Language::Go, a, b, Level::N1);
    assert_eq!(single(Language::Go, a).name, "Push");
}

#[test]
fn kotlin_functions() {
    let a = "fun greet(name: String): String { return \"Hello, \" + name }";
    let b = "fun hello(who: String): String {\n  return \"Hi, \" + who\n}";
    assert_same(Language::Kotlin, a, b, Level::N1);
    assert_eq!(single(Language::Kotlin, a).name, "greet");
}

#[test]
fn ruby_methods() {
    let a = "def total(items)\n  items.sum { |i| i.price }\nend\n";
    let b = "def amount(list)\n  # sum\n  list.sum { |e| e.price }\nend\n";
    let fa = extract_source(Language::Ruby, a.as_bytes(), &config()).unwrap();
    let fb = extract_source(Language::Ruby, b.as_bytes(), &config()).unwrap();
    assert_eq!(fa.len(), 2, "method and block");
    assert_eq!(
        fa[0].exact_hashes[&Level::N1],
        fb[0].exact_hashes[&Level::N1]
    );
    assert_eq!(fa[1].parent, Some(0));
}

#[test]
fn fingerprints_only_for_large_functions() {
    let cfg = Config {
        min_tokens: 1000,
        ..Config::default()
    };
    let f = &extract_source(Language::Java, JAVA_A.as_bytes(), &cfg).unwrap()[0];
    assert!(!f.is_indexed());
    let f = &extract_source(Language::Java, JAVA_A.as_bytes(), &config()).unwrap()[0];
    assert!(f.is_indexed());
    assert_eq!(f.features[&Level::N1].minhash.len(), 128);
    assert_eq!(f.features[&Level::N1].lsh_buckets.len(), 32);
}

#[test]
fn functions_with_syntax_errors_are_skipped() {
    let src = "class A { void ok() { return; } void broken() { int x = ; } }";
    let functions = extract_source(Language::Java, src.as_bytes(), &config()).unwrap();
    let names: Vec<_> = functions.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["ok"]);
}
