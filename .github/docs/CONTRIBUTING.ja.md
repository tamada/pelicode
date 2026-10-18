# pelicode への貢献

[English](../../CONTRIBUTING.md) | 日本語

pelicode に関心を持っていただきありがとうございます。このガイドでは、プロジェクトの構成、ビルドとテストの方法、言語やパッケージリポジトリを追加するときの注意点を説明します。pelicode の目的と背景は [README](README.ja.md)、[利用シナリオ](scenario.ja.md)、[仕様](spec.ja.md) を参照してください。

pelicode が示すのは**確認すべき候補**であり、盗用やライセンス違反の判定ではありません。コード、メッセージ、ドキュメントのいずれでもこの立場を保ってください。

## 開発環境

- [Rust](https://www.rust-lang.org/tools/install) 1.90 以降と C コンパイラ（Tree-sitter の文法と同梱の SQLite は C からビルドされます）。
- [Git](https://git-scm.com/)。`pelicode scan --git` とテストで使います。
- [just](https://just.systems/)。以下のタスクの実行に使います。
- 任意：ドキュメントサイト用の [Hugo](https://gohugo.io/) extended 0.163〜0.166、コンテナイメージ用の Docker、カバレッジ用の [cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov)、ワークフロー確認用の [actionlint](https://github.com/rhysd/actionlint)。

| タスク | 内容 |
| --- | --- |
| `just test` | フォーマット、clippy、ビルドのあと、カバレッジ付きでテスト（`target/coverage.lcov`） |
| `just test_without_coverage` | 同じ内容をカバレッジなしで |
| `just clippy` | `cargo clippy --workspace --all-targets -- -D warnings` |
| `just verify_format` | CI と同じ `cargo fmt --all --check` |
| `just doc` | API ドキュメントの生成。リンク切れなどの警告で失敗します |
| `just publish_check` | `cargo publish --workspace --dry-run`（作業ツリーがクリーンである必要があります） |
| `just docker` | コンテナイメージのビルド |
| `just site` / `just start` | ドキュメントサイトのビルド／ローカルでの表示 |

テストはネットワークに接続しません。接続するコマンド（`pelicode crawl`）は手で試してください。その際は公開サービスに負荷をかけないよう注意してください（[パッケージリポジトリの追加](#パッケージリポジトリの追加) を参照）。

## 構成

リポジトリは Cargo のワークスペースで、各 crate はルート直下のディレクトリです。

```
core  ←  db  ←  detect  ←  cli
          ↑                  ↑
          └──── crawl ───────┘
```

| crate | 役割 |
| --- | --- |
| `core`（pelicode-core） | Tree-sitter による構文解析、関数抽出、正規化、ハッシュ、フィンガープリント、MinHash、LSH、ファイルの探索 |
| `db`（pelicode-db） | SQLite への保存、検索、互換性の確認、ハッシュの集計、不要データの削除 |
| `detect`（pelicode-detect） | 候補の検索、類似度、検査対象の依存関係 |
| `crawl`（pelicode-crawl） | 公開リポジトリからのパッケージの取得 |
| `cli`（pelicode-cli） | `pelicode` コマンド、設定ファイル、git の差分、出力形式、SARIF |

そのほか、`action.yml` と `action/`（GitHub Action）、`.github/workflows/`（CI とリリース）、`.github/docs/`（仕様と利用シナリオ）、`docs/`（Hugo サイト）、`Containerfile`、`Justfile` があります。

## 変更の進め方

1. [issue](https://github.com/tamada/pelicode/issues) を立てるか選んでください。修正より大きな変更は、先に方向性を相談できるようにするためです。
2. ブランチで作業してください。`main` はプルリクエスト経由でのみ変更します。
3. コミットはまとまりごとに分け、件名は命令形（"Add ..."、"Fix ..."）で、本文には何をしたかだけでなく理由を書いてください。コミットかプルリクエストに `Closes #N` と書くと、マージ時に issue が閉じます。
4. プルリクエストを作る前に `just test` と `just doc` を実行してください。CI では、Linux、macOS、Windows（x64 と arm64）でフォーマット、clippy、ビルド、テストを行い、サンプルリポジトリで Action を動かします。
5. 変更に関係するドキュメントを更新してください（[ドキュメント](#ドキュメント) を参照）。

### コーディングスタイル

- `cargo fmt` に従い、clippy の警告をなくしてください（`-D warnings`）。
- コード、コメント、メッセージ、ドキュメントは英語で書きます。README、仕様、利用シナリオには日本語版（`.github/docs` の `*.ja.md`）もあるので、一緒に更新してください。
- コメントは理由を控えめに書き、何をするかは名前で表してください。
- 記録・出力するパスは、どのプラットフォームでも `/` 区切りにします（`pelicode_core::slash_path`）。
- エラーには、何がどこで失敗したか（ファイル、URL、設定ファイルのキー）を含めてください。

## 言語の追加

言語は、Tree-sitter の文法と、そのノードの種類を指定する `LanguageSpec` からなります。

1. `core/Cargo.toml` に文法の crate を追加します（公式の `tree-sitter-*` を優先し、使用中の `tree-sitter` のバージョンで動くか確認してください）。
2. `core/src/language.rs` で、
   - `Language` にバリアントを追加し、`Language::ALL`、`name()`、`extensions()`、`ts_language()`、`spec()` にも追加します。
   - `static` な `LanguageSpec` を追加します。

| フィールド | 意味 |
| --- | --- |
| `functions` | 関数単位となるノードの種類（関数、メソッド、コンストラクタ、ラムダ、ブロック、クロージャ）。本体のない宣言は関数ではありません |
| `comments` | コメントのノードの種類。すべてのレベルで取り除きます |
| `identifiers` | 変数などの識別子。N1 から `$ID` になります |
| `type_identifiers` | 型名。N1 では残り、N2 で `$TYPE` になります |
| `type_contexts` | 通常の識別子を型名として扱う祖先ノード（Python の `type`） |
| `strings`、`numbers` | リテラル。N1 から `$STR`、`$NUM` になります。文字列は 1 トークンです |
| `docstrings` | 本体の最初の文字列文を取り除くか |
| `optional_semicolons` | `;` が書式（Go、Kotlin、Python、Ruby）か構造（Java、Rust）か |

   ノードの種類は、文法の `src/node-types.json` を見るか、サンプルを構文解析して調べてください。
3. `core/tests/extract.rs` にテストを追加します。関数が正しい名前と行で見つかること、識別子の名前を変えると N0 のハッシュは変わるが N1 は変わらないこと、コメントが無視されること、入れ子の関数に親があること、本体のない宣言が除かれることを確認してください。
4. エンドツーエンドのテストに役立つなら、`cli/tests/fixtures/project/` にその言語のファイルを追加します（テストの期待値も更新してください）。
5. その言語にパッケージリポジトリがあれば次の節を参照してください。ロックファイルがあれば、`detect/src/dependencies.rs` で読めるようにしてください。
6. README、サイト（`docs/content/`）、仕様（§4.1）の言語の表を更新します。

言語の追加は既存のハッシュを変えないので、`NORMALIZER_VERSION` はそのままです。

## パッケージリポジトリの追加

リポジトリのアダプタは `crawl/src/` のモジュール（`pypi.rs`、`crates.rs` を参照）で、次のものを持ちます。

- `ECOSYSTEM`：DB に記録する名前（`pypi`、`crates` など）。
- `resolve(client, name, version)`：リリース（または最新版）のメタデータを取得し、`Release` を返します。`Release` は、`PackageInfo`（リポジトリでの表記どおりの名前、バージョン、できるだけ SPDX のライセンス、公開日時、リポジトリの URL、アーカイブの URL）、`ArchiveKind`、取り除く先頭のパス要素の数、リポジトリが公開している `Digest` からなります。
- `parse_release(json)`（または同等のもの）：ネットワークから切り離した解析処理。オフラインでテストできるようにするためです。
- `top_packages(client, n)`：リポジトリがダウンロード数の順位を提供している場合。

そのうえで、

1. `crawl/src/lib.rs` の `Ecosystem`（`name()`、`FromStr`）と `Crawler::resolve` にバリアントを追加し、`cli/src/crawl.rs` のヘルプ（対応していれば `--top` も）に追記します。
2. ダウンロードを検証します。リポジトリが公開するダイジェスト（`Digest`）と照合し、必要なら種類を追加します（`crawl/src/lib.rs` の `verify`）。
3. アーカイブは `crawl/src/archive.rs` がメモリ上で読みます。必要なら形式を追加してください。アーカイブの中身をディスクに書き出したり、ソースコードを保存したりしないでください。
4. メタデータにライセンスがなければ、`license::identify_all` がアーカイブ内のライセンスファイルを読みます。
5. サービスの利用規約を読み、従ってください。リクエストは `Client`（間隔の調整と再試行を行います）を通して一度に一つずつ送り、User-Agent に連絡先（`--contact`）を含め、サービスが求める場合は API ではなくダンプやインデックスを使ってください。
6. テスト：サンプルのレスポンスの解析（`parse_release`）、URL の組み立て、名前。ネットワークには接続しません。ネットワークを使う関数の例は `no_run` にします。
7. README、サイトの Usage、仕様 §6.2 の表に記載します。

## ハッシュと DB のバージョン

二つのバージョンが、互換性のないデータが混ざるのを防いでいます。必要になった変更と同じプルリクエストで上げてください。

- **`NORMALIZER_VERSION`**（`core/src/config.rs`）：既存のコードのハッシュが変わるとき（正規化の規則、関数単位の抽出、トークンやハッシュの計算）に上げます。以前に作った DB は拒否されるようになり、作り直しが必要です。
- **`SCHEMA_VERSION`**（`db/src/lib.rs`）：テーブルを変えるときに上げます。可能なら、前のバージョンからの移行を `check_schema` に追加して（4 から 5 への移行を参照）、既存の DB を使い続けられるようにしてください。
- 特徴量のパラメータ（`kgram`、`window`、`minhash_size`、`bands`、`ident_mode`）は DB ごとに記録されています。既定値を変えてもバージョンを上げる必要はありませんが、ドキュメントにはその旨を書いてください。

## ドキュメント

- **API**：ライブラリの crate は `#![warn(missing_docs)]` です。公開する要素にはすべて doc コメントを、公開する関数にはすべて例を付けてください（ネットワークが必要なら `no_run`）。`just doc` が通る必要があります。
- **コマンドライン**：clap の引数の doc コメントが `--help` の文面になります。短く書き、既定値を明記してください。
- **利用者向け**：README（`README.md`、`.github/docs/README.ja.md`）とサイト（`docs/content/`：Usage、Install、Getting Started、Glossary）。
- **設計**：仕様（`.github/docs/spec.md`、`spec.ja.md`）は実装を説明するものです。機能が入ったら §12.2（未実装の機能）から §12.1（実装状況）へ移してください。利用シナリオ（`.github/docs/scenario*.md`）には、誰に何ができるかを書きます。

## リリース

リリースはメンテナが `release/vX.Y.Z` ブランチから行います。ワークフローがバイナリ、コンテナイメージ、crate をビルドします。手順は準備中です（#10）。

## ライセンス

貢献していただいた内容は、[MIT ライセンス](../../LICENSE) で提供されることに同意したものとします。
