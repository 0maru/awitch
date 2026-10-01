# awitch

GitHub の organization / owner と clone 先のパスに応じて、Claude Code・Codex の
認証プロファイルを選ぶ Rust 製 CLI。macOS / Linux 対応。

`awitch codex` / `awitch claude` で、現在のディレクトリに合ったアカウントを使って起動します。
GitHub API や GitHub のアクセストークンは不要です。

## インストール

Rust 1.89 以降と、使用する `codex` / `claude` CLI をインストールして PATH に置きます。
org の判定には `git` も必要です。

```sh
cargo install --path . --locked
awitch --help
```

## 最初の設定

```sh
awitch init
awitch profile add personal --default
awitch profile add work

# アカウントごと・CLI ごとに一度ログインする
awitch login personal codex
awitch login personal claude
awitch login work codex
awitch login work claude

# GitHub の owner が example-org なら work を選ぶ
awitch rule add work --org example-org

# このディレクトリ以下では work を選ぶ（既存のパスを指定）
awitch rule add work --path ~/workspaces/company
```

以降はリポジトリ内で実行します。

```sh
awitch which                     # 選択結果・理由・保存先を確認
awitch codex                     # 自動選択して起動
awitch claude                    # 自動選択して起動
awitch codex exec 'テストを実行して'
awitch claude -p 'コードを説明して'

awitch --profile personal codex  # 明示指定
awitch --cwd ~/workspaces/company/project claude

awitch profile list
awitch rule list
```

awitch の `--profile` / `--cwd` はサブコマンドの**前**に置きます。
`codex` / `claude` 以降のオプションはそのまま各 CLI に渡します。
たとえば `awitch codex --profile fast` は **Codex 側の設定プロファイル**の指定です。
`awitch codex --help` も Codex のヘルプを表示します。
Codex の `-C` / `--cd` は、アカウントを選ぶ場所と実行先がずれないよう拒否します。
代わりに `awitch --cwd DIR codex ...` を使ってください。

普段のコマンド名のまま使いたい場合は、以下を Bash / Zsh の設定に追加できます。
awitch は PATH 上の実行ファイルを直接起動するため、この関数を再帰呼び出ししません。

```sh
codex() { command awitch codex "$@"; }
claude() { command awitch claude "$@"; }
```

## 選択ルール

| 優先順位 | 条件 |
| --- | --- |
| 1 | awitch の `--profile` |
| 2 | パスルール。より深いディレクトリを優先し、同じ深さでは org 条件ありが優先 |
| 3 | org のみのルール |
| 4 | `default` |

- `--org` と `--path` を同時に指定すると AND 条件になります。
- パスは実体の絶対パスとして保存します。子ディレクトリにも一致しますが、`/work` は `/work-other` に一致しません。
- worktree はその worktree 自身のパスで判定し、remote は Git から取得します。
- org は既定で `origin` の fetch URL の owner。個人アカウント名も指定でき、大文字小文字は区別しません。
- `https://github.com/owner/repo`、`git@github.com:owner/repo.git`、`ssh://git@github.com/owner/repo.git` 等を扱います。
- Git 管理外、Git がない、remote がない、GitHub 以外の場合はパスまたはデフォルトを使います。
- SSH host alias と GitHub Enterprise の org 判定は未対応です。パスルールを使用してください。
- 同順位のルールが異なるプロファイルを指定した場合、または未一致でデフォルトがない場合は起動を止めます。
- 削除されたパスのルールは一致しません。リポジトリを移動した場合はルールも更新してください。

## 認証と保存先

各 CLI はログイン情報をローカルに保存して再利用します。
awitch はプロファイルごとに子プロセスの `CODEX_HOME` / `CLAUDE_CONFIG_DIR` を設定します。
認証情報の読み取り・コピー・入れ替えは行わず、ログインとトークン更新は各 CLI に任せます。
異なるプロファイルを別ターミナルで同時に利用できます。

Codex は `auth.json` または OS の資格情報ストアを使用します。
Claude Code は macOS では通常 Keychain、Linux では `.credentials.json` を使用し、
`CLAUDE_CONFIG_DIR` ごとにログインを分けられます。
仕様の詳細は [Codex Authentication](https://developers.openai.com/codex/auth) と
[Claude Code Authentication](https://code.claude.com/docs/en/authentication) を参照してください。

保存先は `AWITCH_HOME` → `$XDG_CONFIG_HOME/awitch` → `~/.config/awitch` の順に決まります。
プロファイルのディレクトリは最初のログインまたは起動時に作成します。

```text
~/.config/awitch/
├── config.toml
├── config.lock
└── profiles/
    ├── personal/
    │   ├── codex/    # CODEX_HOME
    │   └── claude/   # CLAUDE_CONFIG_DIR
    └── work/
        ├── codex/
        └── claude/
```

設定・履歴・プラグインもプロファイルごとに分離されます。
既存の `~/.codex` / `~/.claude` からは自動移行しません。必要な設定は各プロファイルに追加してください。
Keychain の識別にもパスが使われるため、運用開始後に保存先やプロファイル名を変更した場合は再ログインしてください。
awitch の管理ディレクトリは 0700、`config.toml` は 0600 で作成します。

`OPENAI_API_KEY`、`ANTHROPIC_API_KEY`、`CLAUDE_CODE_OAUTH_TOKEN` など、
保存された認証より優先される既知の環境変数が設定されていると起動を拒否します。
エラーに出た変数を unset して再実行してください。値は表示しません。
対象変数の一覧は [src/launch.rs](src/launch.rs) にあります。
API キーを保存する場合は、対象 CLI が対応する stdin のログインを使用できます。

```sh
printenv OPENAI_API_KEY | env -u OPENAI_API_KEY awitch login work codex --with-api-key
awitch --profile work codex login status
awitch --profile work claude auth status
awitch --profile work codex logout
awitch --profile work claude auth logout
```

対象はローカル CLI の ChatGPT / claude.ai ログインと、各 CLI がプロファイル内に保存する API キーです。
Claude Console の API キーなしログイン（Anthropic profile）は保存先が別なので対象外です。
IDE・デスクトップアプリ、クラウドプロバイダーの認証切り替えは扱いません。
プロジェクト設定・管理者設定・CLI の明示的な設定上書きは引き続き適用されます。
awitch はアカウント選択の補助であり、プロジェクトによる認証上書きを禁止する仕組みではありません。

## 設定ファイル

ルールの削除、デフォルトや remote の変更は `config.toml` を編集します。
awitch のコマンドによる更新はロックと atomic rename を使います。
手動編集と awitch の設定更新コマンドは同時に実行しないでください。
コメントや書式は awitch の設定更新時に再生成されます。

```toml
version = 1
remote = "origin"
default = "personal"
profiles = ["personal", "work"]

[[rules]]
profile = "work"
org = "example-org"

[[rules]]
profile = "work"
path = "/Users/me/workspaces/company"
```

設定ファイル内の `path` は絶対パスにしてください。
`rule add --path` は相対パスや `~/` を展開して保存します。

## 開発

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --locked
```

テストは一時 Git リポジトリと偽 CLI を使い、実際のログインや API リクエストは行いません。
選択順位、worktree、環境変数、ログイン分離、引数、終了コード・シグナル、設定の同時更新を検証します。
設計の詳細は [docs/design.md](docs/design.md) を参照してください。
