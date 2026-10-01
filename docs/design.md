# awitch の設計

## 目的と対象

Rust 製の小さな CLI で、現在のディレクトリと GitHub remote の owner から
Claude Code / Codex の認証プロファイルを選ぶ。初版は macOS / Linux の CLI を対象にする。
GitHub API や GitHub の認証は必要なく、ローカルの Git 設定を使う。

## 認証の扱い

- プロファイルごとに `profiles/<name>/codex` と `profiles/<name>/claude` を作る。
- 子プロセスの `CODEX_HOME` / `CLAUDE_CONFIG_DIR` に絶対パスを設定する。
- ログイン、保存、トークン更新は各 CLI に任せる。awitch は認証情報を読み取らない。
- 初回はプロファイルごとにログインする。既存のログイン情報の自動コピーは行わない。
- 設定・履歴・プラグインもプロファイルごとに分離される。
- CLI は Unix の exec で起動し、TTY、標準入出力、終了コード、シグナルを維持する。
- 保存先を上書きする環境変数、API キー等の既知の認証上書き変数があれば起動を拒否し、変数名だけを表示する。
  `CODEX_HOME` / `CLAUDE_CONFIG_DIR` 自体は選択したプロファイルのパスで上書きする。
- プロジェクトや管理者の CLI 設定は引き続き適用される。認証・アクセス制御を強制するセキュリティ境界ではない。

## 設定とコマンド

保存先は `AWITCH_HOME`、`XDG_CONFIG_HOME/awitch`、`~/.config/awitch` の順。
`config.toml` はプロファイル名、デフォルト名、ルールのみを持つ。
プロファイル名は ASCII 英数字で始まる英数字・ハイフン・アンダースコアとし、パストラバーサルを防ぐ。
管理ディレクトリは 0700、設定ファイルは 0600。設定更新はロックと atomic rename で保護する。

```text
awitch init
awitch profile add personal --default
awitch profile add work
awitch profile list
awitch rule add work --org example
awitch rule add work --path ~/work/company
awitch rule add work --org example --path ~/work/company
awitch rule list
awitch login work codex [CLI のログイン引数...]
awitch login work claude [CLI のログイン引数...]
awitch which
awitch [--profile work] [--cwd DIR] codex [CLI の引数...]
awitch [--profile work] [--cwd DIR] claude [CLI の引数...]
```

awitch のオプションはサブコマンドより前に置く。起動サブコマンド以降は CLI に渡す。
`--cwd` はルールの判定先と子プロセスの作業ディレクトリを同時に変更する。
Codex の `-C` / `--cd` は判定先との食い違いを防ぐため拒否し、awitch の `--cwd` に誘導する。

## 選択規則

1. `--profile` の明示指定
2. パスルール（最も深いディレクトリを優先。同じ深さなら org 条件ありを優先）
3. org のみのルール
4. デフォルト

パスと org の両方を持つルールは AND 条件。同順位が異なるプロファイルを選ぶ場合はエラー。
未一致かつデフォルトなしの場合もエラーにする。
パスは実在ディレクトリを canonicalize し、文字列前方一致ではなくパス要素で比較する。
worktree では現在の worktree の物理パスを使い、org は Git が返す remote を使う。
GitHub owner は org と個人アカウントを区別せず、大文字小文字を区別しない。
remote は既定で `origin`。設定の `remote` で変更できる。
HTTPS / SSH URL / SCP 形式の `github.com` を扱い、Enterprise・SSH host alias は対象外。
Git 管理外、remote 未設定、GitHub 以外では org が不明としてパス・デフォルトを利用する。

## 検証

ルールの優先順位、境界、曖昧さ、壊れた設定、Git URL の解析をユニットテストする。
一時 Git リポジトリと偽 CLI を使い、実際のコマンドでログイン・選択・起動・環境変数・
引数・終了コード・worktree・同時更新を検証する。実アカウントへのログインや課金リクエストはテストしない。

## 根拠

- [Codex Authentication](https://developers.openai.com/codex/auth): ローカル認証のキャッシュと `CODEX_HOME`。
- [Claude Code Authentication](https://code.claude.com/docs/en/authentication): `CLAUDE_CONFIG_DIR` による複数アカウントと Keychain の分離。
- [Claude Code Environment variables](https://code.claude.com/docs/en/env-vars): 設定ディレクトリの指定。

Claude Console の API キーなしログイン（Anthropic profile）は設定ディレクトリ外に保存されるため初版の対象外。
