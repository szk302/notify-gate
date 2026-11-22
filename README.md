# notify-gate

`ngate` は、[lmm](https://github.com/szk302/local-markdown-mail) に届いたメッセージを、受け取り手に知らせる道具です。知らせる先はアダプタで切り替えます。[tmac](https://github.com/szk302/tmux-multi-agent-controller) のエージェントには、組み込みのアダプタで知らせを入力します。

- 新着を `lmm wait` で待ち、届いたら届け先に知らせます。届け先がいなくなるまで動き続けます。
- 知らせた後も対応されずに残っているメッセージは、1 通につき 1 回だけ知らせ直します（tmac の届け先の既定）。
- `ngate guard` を tmac の中継の `Stop` に置くと、未対応のメッセージが残っている間、エージェントが応答を終えるのを 1 回止めます。
- 知らせの文面は、テンプレートで変えられます。既定では、何通届いたか、どこにあるかだけを伝えます。
- lmm は、lmm のコマンドだけで使います（データディレクトリを直接読みません）。

仕様は [docs/prd.md](docs/prd.md) を参照してください。外部（Slack など）からの返事を lmm に入れる機能は、今後の版で作ります。

## インストール

### ビルド済みのバイナリ

[Releases](https://github.com/szk302/notify-gate/releases) から、環境に合ったファイルをダウンロードして展開し、`ngate` を `PATH` の通った場所に置きます。

| ファイル名の末尾 | 環境 |
| :--- | :--- |
| `x86_64-unknown-linux-musl.tar.gz` | Linux（x86_64、静的リンク） |
| `aarch64-unknown-linux-musl.tar.gz` | Linux（arm64、静的リンク） |
| `aarch64-apple-darwin.tar.gz` | macOS（Apple Silicon） |
| `x86_64-apple-darwin.tar.gz` | macOS（Intel） |

```bash
v=v0.1.0; t=aarch64-unknown-linux-musl
curl -LO "https://github.com/szk302/notify-gate/releases/download/$v/notify-gate-$v-$t.tar.gz"
curl -LO "https://github.com/szk302/notify-gate/releases/download/$v/SHA256SUMS"
grep "notify-gate-$v-$t.tar.gz" SHA256SUMS | sha256sum -c -   # macOS では shasum -a 256 -c -
tar xzf "notify-gate-$v-$t.tar.gz"
mkdir -p ~/.local/bin
install "notify-gate-$v-$t/ngate" ~/.local/bin/
```

アーカイブには、`LICENSE` と、同梱しているサードパーティーのライセンス表示（`THIRD-PARTY-LICENSES.txt`、Linux 版は `MUSL-COPYRIGHT` も）が入っています。

### ソースから

Rust ツールチェーンは [mise](https://mise.jdx.dev/) で管理しています。

```bash
mise install                 # mise.toml の Rust をインストール
cargo install --locked --path .
```

### 必要なもの

lmm（`lmm config paths` のある版）が必要です。tmac のエージェントに知らせる場合は tmac も必要です。

## 使い方

### tmac のエージェントに知らせる

tmac の中継で、エージェントが起動したときと応答を終えるたびに、リンクを起動します（動いていれば何もしません）。`Stop` では、未対応のメッセージが残っていれば応答の終わりを止めます。

```bash
tmac spawn todo-coder \
  --relay 'SessionStart=ngate link --detach todo-coder tmac' \
  --relay 'Stop=ngate link --detach todo-coder tmac; ngate guard todo-coder' \
  -- claude
```

- 届け先の `tmac` は、中継の環境変数（`TMAC_PROJECT`・`TMAC_AGENT`・`TMAC_INSTANCE`）でエージェントを決めます。外からは `tmac:<PROJECT>/<NAME>` で指定できます。
- エージェントが終了するか、起動し直されると、リンクも終わります。

```bash
ngate list                  # リンクの一覧
ngate stop todo-coder       # このディレクトリの lmm のデータディレクトリにある、todo-coder のリンクを止める
```

### 文面を変える

```yaml
# notices.yml
new: "lmm に新着メッセージが {count} 通あります。ルールの「受け取る」に従って対応してください。\n{paths}"
reminder: "対応していない lmm のメッセージが {count} 通残っています。\n{paths}"
guard: "対応が終わっていない受信メッセージがあります。\n{paths}"
```

```bash
ngate link --detach --template notices.yml todo-coder tmac
ngate guard --template notices.yml todo-coder
```

プレースホルダは `{count}`・`{paths}`・`{address}` です。

### ほかの届け先（外部アダプタ）

`$NGATE_HOME/config.yml`（既定 `~/.ngate/config.yml`）に届け先を書き、`ngate-adapter-<名前>` を `PATH` に置きます。

```yaml
interval_sec: 30
destinations:
  desktop:
    adapter: notify-send
    after_delivery: keep
    with: {urgency: low}
```

```bash
ngate link --detach me desktop
```

アダプタは `ngate-adapter-<名前> deliver` の形で呼ばれ、標準入力で JSON（`kind`・`recipient`・`destination`・`messages`・`text`）を受け取り、終了コード（0 届けた、3 あとで、4 相手がいない、5 対応しない操作）で答えます。詳しくは仕様の §5 です。

## 終了コード

| コード | 意味 |
| :--- | :--- |
| 0 | 成功 |
| 1 | 実行時のエラー |
| 2 | 使い方・設定の誤り。`ngate guard` では、未対応のメッセージが残っている |

## 開発

```bash
mise run build
mise run test    # 結合テストは lmm（lmm config paths のある版）と Python 3 を使う
mise run lint
```

結合テスト（`tests/link.rs`）は、テストごとに一時的な lmm のプロジェクトを作り、偽の tmac と偽のアダプタ（呼び出しを記録して、決めた終了コードを返す）を使います。使う lmm は `NGATE_TEST_LMM`（実行ファイルのパス）か、`PATH` の `lmm` です。

## ライセンス

[MIT](LICENSE)
