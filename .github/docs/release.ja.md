# pelicode のリリース

[English](release.md) | 日本語

リリースは `release/vX.Y.Z` ブランチから行う。ワークフローがそのブランチで版を上げ、`main` にマージするとすべてをビルドして公開する。このページでは、準備しておくもの、手順、リリース後に確かめること、失敗したときの対処をまとめる。

## リリースで公開されるもの

| 内容 | 公開先 | 担当 |
| --- | --- | --- |
| git のタグ `vX.Y.Z` と、リリースノート付きの GitHub リリース | [Releases](https://github.com/tamada/pelicode/releases) | `release-start`（tamada/.github の共通ワークフロー） |
| Linux・macOS・Windows の x64 と arm64 のバイナリ（`pelicode-X.Y.Z_<os>_<arch>.tar.gz` または `.zip`） | リリースの添付ファイル | `publish` ジョブ |
| linux/amd64 と linux/arm64 のコンテナイメージ（タグは `X.Y.Z`、`latest`、短いコミット ID） | `quay.io/tama5/pelicode` | `docker` と `docker_manifest` ジョブ |
| 5 つの crate（`pelicode-core`、`-db`、`-detect`、`-crawl`、`-cli`） | crates.io | `publish_to_crates_io` ジョブ |
| Homebrew の formula | `tamada/tap` | `notify-publish` から `tamada/api` 経由 |

ドキュメントサイトは、`main` の `docs/` が変わるたびに `deploy documentation` が公開するので、マージに合わせて更新される（版のバッジもほかと一緒に上がる）。

## 準備しておくもの

リポジトリごとに一度だけ設定する。

| シークレット・設定 | 使うもの | 内容 |
| --- | --- | --- |
| `QUAY_ACCOUNT`、`QUAY_TOKEN` | `docker`、`docker_manifest`、`snapshot` | `tama5/pelicode` と `tama5/pelicode-db` に書き込める quay.io のロボットアカウント |
| `APP_CLIENT_ID`、`APP_PRIVATE_KEY` | `release-finish`、`notify-publish` | GitHub App `tamada-pipline-bot`（リポジトリにインストール済みであること）。このトークンでリリースを公開すると `release: published` が発生し、`notify-publish` が動く |
| `SONAR_TOKEN` | `Sonar Qube` | SonarCloud の解析（リリース自体には不要） |
| Coveralls | `build_products` | coveralls.io でリポジトリを有効にする。`GITHUB_TOKEN` を使う |
| GitHub Pages のソースを「GitHub Actions」に | `deploy documentation` | Settings → Pages |
| crates.io の Trusted Publishing | `publish_to_crates_io` | 5 つの crate それぞれに、リポジトリ `tamada/pelicode`、ワークフロー `publish.yaml` を登録する。[初回のリリース](#初回のリリース) を参照 |
| quay.io の公開リポジトリ `tama5/pelicode` と `tama5/pelicode-db` | イメージとスナップショット | quay.io は最初の push でリポジトリを非公開で作るので、一度だけ公開に変える |

## 手順

1. **`main` を確かめる。** すべてのプラットフォームで CI（`build_products`）が通っていること、ドキュメント（README、サイト、仕様の §12）がリリースする内容を説明していることを確かめる。

2. **リリースノートを書く**（任意だが推奨）。`.github/release-notes/vX.Y.Z.md` を、リリースの前に `main` にマージするか、リリースブランチに追加する。ない場合は、GitHub が PR からリリースノートを生成する。

3. **リリースブランチを作る。** `main` から作って push する。

   ```sh
   git switch -c release/vX.Y.Z main
   git push -u origin release/vX.Y.Z
   ```

   `update_version` が、このブランチに「update version to X.Y.Z, ready to publish vX.Y.Z」をコミットする。このコミットで、次の版が上がる。
   - `Cargo.toml`：`[workspace.package]` と、crate 同士が求める版（`[workspace.dependencies]`）
   - `Cargo.lock`
   - `docs/content/_index.md` の版のバッジ
   - `Containerfile` の `ARG VERSION`
   - `CITATION.cff` の `version` と `date-released`

4. **PR を作る。** そのコミットが入ったのを確かめてから（`git pull` で確認できる）、`release/vX.Y.Z` から `main` への PR を作り、CI を待つ。

5. **マージする。** マージで `publish_site_release_assets` が動く。
   1. `notes` が、リリースノートがあれば見つける。
   2. `setup` が `main` に `vX.Y.Z` のタグを付け、下書きのリリースを作る。
   3. `publish` が 6 つのバイナリをビルドし、下書きに添付する。
   4. `docker` が amd64 と arm64 でそれぞれイメージをビルドし、`docker_manifest` が両方をまとめたイメージにタグを付ける。
   5. `publish_to_crates_io` が、依存の順に crate を公開する。
   6. `finish` が、App のトークンでリリースを公開する。これで `notify-publish` が動き、Homebrew の formula が更新される。

6. **リリースを確かめる**（次の節）。

## リリース後に確かめること

- リリース `vX.Y.Z` が公開されていて（下書きではなく）、6 つのアーカイブが添付されている。各アーカイブには `pelicode`（または `pelicode.exe`）、`README.md`、`LICENSE` が入っている。
- アーカイブの `pelicode --version` が `X.Y.Z` を表示する。
- amd64 と arm64 で `docker run --rm quay.io/tama5/pelicode:X.Y.Z --version` が動く。
- `cargo install pelicode-cli` で `X.Y.Z` が入る。
- `brew upgrade tamada/tap/pelicode`（または `brew install`）で `X.Y.Z` が入る。
- サイトの版のバッジが `X.Y.Z` を示し、GitHub の「Cite this repository」に版と日付が出る。

## 初回のリリース

crates.io の Trusted Publishing は、すでに存在する crate にしか設定できない。そのため、最初の版の crate は、API トークンを使って手で公開する。

1. 手順 4 のあと（リリースブランチで版が上がったあと）、リリースブランチの先頭から公開する。

   ```sh
   git switch release/vX.Y.Z && git pull
   cargo login                      # crates.io の API トークン（publish の権限が必要）
   cargo publish --locked --workspace
   ```

2. crates.io で、5 つの crate それぞれに Trusted Publishing を設定する（Settings → Trusted Publishing。owner `tamada`、repository `pelicode`、workflow `publish.yaml`）。API トークンが不要になったら取り消す。

3. PR をマージする。`publish_to_crates_io` は、その版の crate がすべて crates.io にあることを確かめて、公開を飛ばす。次からのリリースは、Trusted Publishing で公開される。

## 失敗したとき

- **`publish_site_release_assets` のジョブが失敗した**：Actions の画面から、失敗したジョブを再実行する。下書きのリリースとタグは残っている。`publish` は `--clobber` で添付するので、再実行すれば添付ファイルは置き換わる。
- **全体をやり直したい**：`publish_site_release_assets` を手動で実行し（`workflow_dispatch`）、`0.1.0` のように版を入力する。`main` で動くので、版を上げる変更はマージ済みである必要がある。
- **一部の crate だけが公開された**：`cargo publish --workspace` は公開済みの crate で失敗するので、`publish_to_crates_io` は止まる。残りの crate を依存の順に手で公開し（`cargo publish -p pelicode-db` など）、ジョブを再実行する。
- **リリースが下書きのまま、または Homebrew が更新されない**：`finish` は、ほかのジョブがすべて成功したときだけ下書きを公開する。`notify-publish` は、App のトークンで公開されたときだけ動く。`APP_CLIENT_ID` と `APP_PRIVATE_KEY` を確かめてから、下書きを手で公開し、`notify-publish` を手動で実行する（`workflow_dispatch`）。
- **公開した crate の版は取り消せない**：crates.io でできるのは yank（`cargo yank --version X.Y.Z -p <crate>`）だけである。修正版を出して対応する。
