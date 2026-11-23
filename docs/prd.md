# notify-gate 仕様書

| 項目 | 内容 |
| :--- | :--- |
| 対象バージョン | v0.1.0 |
| 状態 | 外向き（知らせる）と tmac アダプタを実装した。人向けのリンク（`ngate ensure`）、内向き（外部からの返事）、Slack アダプタは未実装 |
| 実装言語 | Rust |
| 関連 | [lmm](https://github.com/szk302/local-markdown-mail)、[tmac](https://github.com/szk302/tmux-multi-agent-controller) |

---

## 1. 目的と位置づけ

* lmm に届いたメッセージを、受け取り手に知らせる（**外向き**）。受け取り手が tmac のエージェントなら tmac で入力し、人なら Slack などに送る（Slack は今後）。
* 知らせる先の違いは、アダプタで吸収する。新しい先を使いたくなったら、アダプタ（外部コマンド）を書けばよい（§5）。
* 外部での返事を lmm のメッセージにする**内向き**は、今後の版で作る。

### 1.1. ほかの道具との分担

| 道具 | 役割 |
| :--- | :--- |
| lmm | メッセージを保存する。誰がどこにいるかは知らない |
| notify-gate | 届いたことを受け取り手に知らせる |
| tmac | エージェントを操作する（起動、状態、入力、終了） |
| 呼び出し側（エージェントを動かすワークフローなど） | 道具どうしをつなぐ（tmac の中継に notify-gate を設定する、知らせの文面を渡すなど） |

* 依存の向きは、notify-gate → lmm と、tmac アダプタ → tmac だけとする。lmm と tmac は notify-gate を知らない。本体は tmac を知らない（tmac の環境変数を読むのも、tmac アダプタだけである）。
* notify-gate は、lmm の**利用者の 1 つ**として、lmm のコマンド（`lmm config paths`、`lmm wait`、`lmm show --json`、`lmm box list`）だけを使う。lmm のデータディレクトリを直接読み書きせず、lmm の設定の決め方も写さない。
* 知らせの文面には、メッセージへの対応の手順（読み方、返信、削除）を書かない。手順は受け取り手のルールに書く。文面を変えたいときは、呼び出し側がテンプレートを渡す（§6）。

### 1.2. 必要なもの

* lmm 0.3.0 以降（`lmm config paths` がある）
* tmac アダプタを使う場合は tmac
* 環境変数 `NGATE_LMM`・`NGATE_TMAC` で、使う lmm・tmac の実行ファイルを変えられる（既定は `PATH` の `lmm`・`tmac`）。

---

## 2. リンク

リンクは、受け取り手（lmm のアドレス）1 つと届け先 1 つをつなぐ、notify-gate の実行の単位である。

```bash
ngate link [--detach] [--template <FILE>] <ADDRESS> <DESTINATION>
ngate stop <ADDRESS>
ngate list
```

* `<DESTINATION>` は次のどれかとする。
    * `tmac`: tmac の中継から起動したときのエージェント。環境変数 `TMAC_PROJECT`・`TMAC_AGENT`・`TMAC_INSTANCE` で決める。
    * `tmac:<PROJECT>/<NAME>`: tmac のエージェントを名前で指定する。
    * 設定の `destinations.<NAME>`（§7）。外部アダプタを使う。
* リンクは、**lmm のデータディレクトリとアドレスの組**ごとに 1 つだけ動く（ロックで守る）。2 つ目の起動は、何もせずに終了コード 0 で終わる。データディレクトリは、カレントディレクトリでの `lmm config paths`（`LMM_NAME` に `<ADDRESS>` を設定）で決める。
* `--detach` は、切り離して（新しいセッションで）起動し、すぐに戻る。tmac の中継から呼ぶときに使う。届け先と設定の誤りは、切り離す前に確かめて報告する。
* `--template` は、知らせの文面のテンプレート（§6）。設定の届け先の `template` より優先する。
* リンクは、カレントディレクトリで lmm のコマンドを実行する。展開先などは、lmm の決め方による（受け取り手の作業ディレクトリで起動すれば、そこでの lmm の設定が使われる）。
* `ngate stop` は、カレントディレクトリでのデータディレクトリと `<ADDRESS>` の組のリンクを止める。リンクのプロセスグループに `SIGTERM` を送り、10 秒たっても止まらなければ `SIGKILL` を送る。動いていなければ、何もせずに終了コード 0 で終わる。
* `ngate list` は、記録のあるリンクを表で示す（アドレス、届け先、`running`・`stopped`、プロセス ID、作業ディレクトリ）。

### 2.1. 動き

1. tmac のエージェントを名前で指定した場合は、そのときのインスタンス ID を `tmac status` で記録する。エージェントがなければ終わる。
2. 届け先がまだいるかを確かめる（§5 の `alive`）。いなければ終わる。
3. `lmm wait --all --timeout <interval_sec>` で、新着を待って展開する。
    * 時間切れなら、手順 2 に戻る。
    * 失敗した場合は記録する。メールボックスがなくなっていれば（`lmm box list` にない）終わる。そうでなければ、`interval_sec` 待って手順 2 に戻る。
4. 展開した各メッセージの値を `lmm show --json` で読み、届け先に知らせる（`deliver`、`kind: new`）。
    * 「今は届けられない」か失敗なら、`interval_sec` 待ってやり直す。「相手がいない」なら終わる。
5. **届けた後:** 届け先の `after_delivery` が `delete` なら、展開したディレクトリを消す（原本は lmm に残る）。`keep`（既定）なら残す。
6. **知らせ直し:** 届け先の `remind` が `once` なら、知らせた後も展開先に残っているディレクトリを、1 通につき 1 回だけ、`kind: reminder` で知らせ直す。手順 2 に戻る。
    * tmac のエージェントでは、`tmac wake` は、作業中ならその作業の終わり（`Stop`）を待つだけで入力しない。入力待ちなら知らせ直しを入力する。応答を終えたのに残っているメッセージを、見落とさないようにするためである。

* `SIGTERM`・`SIGINT`・`SIGHUP` で終わる。実行中の lmm やアダプタにも `SIGTERM` を送る。
* 待ち時間は、設定の `interval_sec`（既定 30 秒）とする。

---

## 3. `ngate guard`

```bash
ngate guard [--template <FILE>] [<ADDRESS>]
```

tmac の中継の `Stop` から呼び、未対応のメッセージが残っていれば、エージェントが応答を終えるのを止める。

* 環境変数 `TMAC_STOP_ACTIVE` が `true`（中継が止めた後の続き）なら、何もせずに終了コード 0 で終わる。止めるのは 1 回の応答につき 1 回だけで、エージェントが止まらなくなることはない。
* `<ADDRESS>` を省略した場合は、lmm の自分のアドレス（`lmm config paths` の `name`）とする。決まらなければ、何もせずに終了コード 0 で終わる。
* 展開先（`lmm config paths` の `receive_dir`）に、`.` で始まらないディレクトリが残っていれば、理由（§6 の `guard`）を標準エラー出力に出して、終了コード 2 で終わる。なければ 0 で終わる。
* CLI の形式（`{"decision": "block"}` など）は使わない。tmac は、中継の終了コード 2 と理由を、そのまま CLI に返す（Claude Code と Codex は、どちらもこの形を受け付ける）。
* 使うかどうかは、中継を設定する側が決める（人への質問で応答を終えるエージェントには使わない、など）。

---

## 4. tmac アダプタ（組み込み）

| 操作 | 内容 |
| :--- | :--- |
| `deliver` | `tmac wake <PROJECT>/<NAME> <文面>`。終了コードは、0 → 届けた、4 → 相手がいない、6（入力を確かめられず tmac が保留した）→ 届けたとする（人が確かめる。もう一度入力しない）、それ以外 → あとでやり直す |
| `alive` | `tmac status <PROJECT>/<NAME> --field instance` が記録のインスタンス ID と同じで、`--field state` が `exited` でなければ、いる。違えば（起動し直された、終了した、ない）、いない。`tmac status` が失敗した場合も、いないとする（リンクがエージェントより長く残らないようにするため） |

* tmac の届け先では、`after_delivery` は `keep` に限る（`delete` は設定の誤り）。エージェントは対応が終わったら自分でディレクトリを消し、知らせ直しと `ngate guard` は、残っているディレクトリを手がかりにするためである。
* `remind` は `once` とする。

---

## 5. 外部アダプタ

* `ngate-adapter-<NAME>` という名前の実行ファイルとし、`PATH` から探す。見つからなければ、リンクは起動しない（終了コード 2）。
* `ngate-adapter-<NAME> <OP>` の形で呼び、標準入力に JSON を 1 つ渡す。

| 操作 | 必須 | 入力 |
| :--- | :--- | :--- |
| `deliver` | 必須 | `{"kind": "new"\|"reminder", "recipient": <アドレス>, "destination": {"adapter": <NAME>, "with": <設定の with>}, "messages": [...], "text": <文面>}` |
| `alive` | 任意 | `{"recipient": <アドレス>, "destination": {...}}`。待つたびに呼ぶ |

* `messages` の各要素は、`lmm show --json` から作る `{"path", "id", "from", "to", "subject", "thread_id", "in_reply_to", "created_at"}` とする（値がなければ `null`）。読めなかったメッセージは `{"path"}` だけとする。
* 終了コード:

| 終了コード | 意味 |
| :--- | :--- |
| 0 | 届けた（`alive` では、いる） |
| 3 | 今は届けられない。あとでやり直す |
| 4 | 相手がいない。リンクを終える |
| 5 | その操作に対応していない（`alive` に対応しないアダプタは、いるとみなす） |
| その他 | 失敗。記録して、あとでやり直す |

* 秘密の値（トークンなど）は、設定に書かず、アダプタが環境変数や秘密のファイルから読む。notify-gate は記録に秘密の値を書かない。

---

## 6. 知らせの文面（テンプレート）

YAML のファイルで、次のキーの文字列を変えられる。どのキーも省略でき、省略したものは既定の文面になる。

| キー | 使う場面 | 既定 |
| :--- | :--- | :--- |
| `new` | 新着の知らせ | `{count} new lmm message(s) for {address}:` と `{paths}` |
| `reminder` | 知らせ直し | `{count} lmm message(s) for {address} are still left:` と `{paths}` |
| `guard` | `ngate guard` の理由 | `lmm messages you have not dealt with are left:` と `{paths}` |

* プレースホルダは `{count}`（件数）、`{paths}`（`- <パス>` を 1 行ずつ）、`{address}`（受け取り手）とする。`{` と `}` そのものは `{{`・`}}` と書く。知らないプレースホルダと、知らないキーは誤りとする。
* 既定の文面は、何通届いたか、どこにあるかだけを示す。対応の手順は、受け取り手のルールに書く（§1.1）。

---

## 7. 設定

`$NGATE_HOME/config.yml`（`NGATE_HOME` の既定は `~/.ngate`）。ファイルがなければ既定値を使う。

```yaml
interval_sec: 30
destinations:
  desktop:
    adapter: notify-send      # ngate-adapter-notify-send を呼ぶ
    remind: never
    after_delivery: keep
    template: desktop.yml     # 相対パスは $NGATE_HOME から
    with: {urgency: low}
```

| キー | 既定 | 内容 |
| :--- | :--- | :--- |
| `interval_sec` | `30` | 待つ時間の上限と、やり直すまでの間隔（秒） |
| `destinations.<NAME>.adapter` | （必須） | アダプタの名前（§5） |
| `destinations.<NAME>.remind` | `never` | `once` なら知らせ直す（§2.1 の手順 6） |
| `destinations.<NAME>.after_delivery` | `keep` | `delete` なら、届けた後に展開したディレクトリを消す |
| `destinations.<NAME>.template` | なし | 知らせの文面（§6） |
| `destinations.<NAME>.with` | なし | アダプタに渡す値 |

* `remind: once` と `after_delivery: delete` は同時に指定できない。知らせ直しは、残っているディレクトリを手がかりにするためである。
* `tmac` と `tmac:` で始まる名前は、届け先の名前に使えない。
* 知らないキーは誤り（終了コード 2）とする。ただし、今後の版で使う `links`（`ngate ensure`）と `destinations.<NAME>.allow`（内向き）は、警告を出して無視する。

---

## 8. 状態

`$NGATE_HOME/links/<ADDRESS>-<HASH>/`（`HASH` はデータディレクトリの絶対パスの SHA-256 の先頭 12 桁）に、リンクごとに次を置く。

| ファイル | 内容 |
| :--- | :--- |
| `link.yml` | アドレス、届け先、データディレクトリ、作業ディレクトリ、プロセス ID と起動日時（動いている間）、終わった日時と理由 |
| `lock` | 動いているリンクが排他ロックを持つ |
| `log` | 受け取り、届け、やり直し、終わりの記録（日時付き、1 行に 1 件） |

---

## 9. 終了コード

| コード | 意味 |
| :--- | :--- |
| 0 | 成功（`ngate guard` では、止めない） |
| 1 | 実行時のエラー |
| 2 | 使い方・設定の誤り（`ngate guard` では、未対応のメッセージが残っている） |

---

## 10. 今後

* 人向けのリンクを、設定の `links` から起動する `ngate ensure`
* 内向き（外部の返事を `lmm post --reply` にする。許可した相手だけを通す）
* Slack アダプタ（外向き、内向き）
