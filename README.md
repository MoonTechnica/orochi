# Orochi

**Adaptive Agent & Model Scheduler** — ACP Coding Agentを、タスク・利用制限・Provider Policy・ローカル実績から選ぶRust CLI。

```sh
orochi "認証処理をrefresh token方式に変更して"
orochi   # 端末で起動すると対話セッション（orochi -c で直近の会話を再開）
```

## 現在の実装範囲

[元のDraft仕様](docs/design-draft.md)を段階的に実装しています。通常実行は1つのAgentへ委譲し、`collaborate`は独立した複数セッションで実装・レビュー・統合を行います。

- Claude / Codex / Gemini / AntigravityのCLI検出と、任意のACP stdioアダプター設定
- Claude/Codex本体の検出、不足するACPアダプターの専用キャッシュへの自動導入
- 公式Rust ACP SDK 2.0.0によるstable protocol v1接続・ストリーミング・キャンセル
- `configOptions`のモデル・reasoning・mode取得。モデル変更後に返された設定を再取得・検証
- 旧式の`models` / `session/set_model`と`modes`の互換処理。設定がないAgentではAgent自身のデフォルトを利用
- ローカルTask Profiler、決定的ルーティング、成功確率の下限、Provider hard constraints
- 任意設定の判定役（ACPエージェント）。候補IDだけを推薦し、Schedulerが再検証
- Agent全体／モデル別cooldown、reset時刻、5分→15分→30分→60分のbackoff
- 利用残量が取得できた場合のquota shadow price、履歴統計、cache affinityの推定
- TaskEnvelopeによるfallback。変更中のファイルを保持し、会話全文は転送しない
- テスト・typecheck・lint・build・git diffによる評価、SQLiteローカルtelemetry
- セッション一覧、明示的な`--resume`、Policyの検証・アトミック更新
- 同一リポジトリの排他実行、子プロセス終了待ち、Unixでのプロセスグループ終了

EWMAと制約付きBandit、予測校正・候補比較、Codex残量の直接取得、Claude statusline残量の取り込み、FrontierのルーティングJudge、2ラウンドのルーティングCouncil、`orochi serve`によるACP公開を追加しました。詳細・設定・未検証部分は[適応ルーティングとACP Gateway](docs/adaptive-routing.md)を参照してください。

[実測・実機検証結果](docs/real-validation-20260916.md): Claude/Codexの同一課題8組の実測、学習係数のholdout検証、Claudeの随時残量取得、同じClaude Haikuの独立3セッションによる実装・レビュー・統合、公開ACPから実Codexへのタスク実行を確認しました。今回の小規模データではEWMA/Banditの優位性は確認できず、既定係数は維持しています。

[セッション協議](docs/session-collaboration.md)は`orochi collaborate`で使用します。同じAgent・モデルでもセッションが異なれば別の作業者です。司令塔・実装・レビュー・統合の全役割で、利用制限時に利用可能な別Agent／モデルへ交代します。計画・判断・途中回答・進捗を`report.json`へ保存し、全候補が使えない場合は`collaborate-resume`で後から再開できます。最大4人の並列実装とマージ、任意の参加者への宛先付きメッセージと協議ラウンドに対応します。`--apply`を指定すると、検証に成功した結果を元の作業ツリーへマージし、競合は解消用のセッションで解決します。

[エージェント間メールボックス](docs/agent-mailbox.md)により、複数のターミナルで同時に動かした`orochi`のエージェント同士が、同じリポジトリ（worktreeを含む）の作業についてメッセージをやり取りできます。`scheduler.shared_workspace = true`で同じディレクトリでの同時実行も可能です。実Codex同士で、関数の仕様を伝えて実装を合わせることを確認しました。

Agentは監視プロセス経由で起動します。Orochiが強制終了されても、Agentと子孫プロセスを終了します。監視プロセスごと止まった場合は、次回起動時に回収します。

Router／Judge／Councilの判定役も`[[agents]]`のエージェントをACPで使います（HTTP endpoint版は削除）。Orochiは認証情報を扱わず、各CLIの認証（サブスクリプションのログインまたはAPIキー）に従います。ACP対応のCLIがローカルモデルに対応していれば、実行・判定役のどちらにも使えます。

[残タスクの実機検証](docs/real-validation-20260916-2.md)で次を確認しました。

- 実Codexの利用上限から実Claudeへの交代
- 実Claude／Codexによる並列実装・協議・作業ツリーとの競合解消
- 強制終了からの回収と再開
- ACP判定役の3方式での交代
- Zedのエージェントパネルからの実行
- 言語・難易度の異なる6課題の実測

実Claudeの利用上限から実Codexへの交代は未確認です。Antigravityの実機E2EはGoogleログイン待ちです。Gemini CLIの追加検証はユーザー指示により対象外にしました。ローカルモデルでの実機確認はまだ行っていません。

## ビルド

Rust **1.96.0**、C/C++ビルドツールが必要です。SQLiteは同梱ビルドします。

```sh
cargo build --release --locked
cargo install --path . --locked
orochi --help
```

Rustは`rust-toolchain.toml`で固定しています。miseを使っている場合は、このディレクトリで同じRustバージョンを選択してください。

## Agentの準備

使用するCLIをインストールし、**各Agent自身の公式ログイン手順**で認証してください。Orochiは認証情報の収集やログイン代行を行いません。

| Agent ID | 検出・接続方法 | 導入元 |
|---|---|---|
| `codex` | `codex`本体、または`codex-acp`。不足するアダプターは自動導入 | [Codex ACP](https://github.com/agentclientprotocol/codex-acp) |
| `claude` | `claude`本体、または`claude-agent-acp`。不足するアダプターは自動導入 | [Claude Agent ACP](https://github.com/agentclientprotocol/claude-agent-acp) |
| `gemini` | `gemini --acp` | [Gemini CLI ACP mode](https://geminicli.com/docs/cli/acp-mode/) |
| `antigravity` | `agy_acp_server.par` | [Antigravity ACP Registry掲載情報](https://zed.dev/acp/agent/antigravity-acp) |

Claude/CodexはCLI本体だけがインストールされていても検出されます。アダプターがない場合はNode.js 22以上とnpmを使い、初回のdiscovery時に準備します。

```sh
orochi config init
orochi agents
orochi agents --discover
```

プリセットはPATH上の実行ファイルを検出します。既存ACPコマンドを優先し、不足分はデータディレクトリの`adapters/`へ固定バージョン（Codex ACP 1.10.0、Claude Agent ACP 0.77.0）で導入します。グローバルインストールや対象リポジトリの依存関係は変更しません。導入は公式npmレジストリから行い、install scriptsを無効化し、完成したキャッシュを再利用します。CLI本体は`CODEX_PATH` / `CLAUDE_CODE_EXECUTABLE`でアダプターに渡します。明示されたenv設定は維持します。

CLIがPATHにない場合は`command`に絶対パスを設定できます。Claude/Codexのネイティブ実行ファイルはファイル名`claude` / `codex`で識別します。独自のACPコマンド・引数はそのまま使います。GeminiはネイティブACP、AntigravityはACP実行ファイルを使用します。その他のAgentは`[[agents]]`にACPコマンドを登録してください。

既知の4プリセットは設定ファイルに省略されていても補完されます。同じIDの明示設定を優先するので、`enabled = false`で除外できます。自動補完やダウンロードを制御する場合:

```toml
[discovery]
auto_add = true          # falseなら[[agents]]に書いたものだけ使用
auto_install = true     # falseなら不足アダプターを自動導入しない
setup_timeout_secs = 120 # ACP接続のタイムアウトとは別枠
```

`orochi agents` / `status`はダウンロードしません。`ready`は接続準備済み、`adapter_required`はCLI本体検出済みで初回準備待ちです。`setup_unavailable`はNode/npm不足、`setup_disabled`は自動導入無効を意味します。認証やACP接続の失敗は`agents --discover`とルーティング時に別途表示されます。

`agents --discover`と`--dry-run`は必要なアダプターを準備し、ACPプロセスと一時セッションを作成して設定を取得します。タスクのpromptは送信しません。インストール済みであっても、認証・実行権限・ACP互換性に問題があれば失敗理由を表示します。

## 使い方

```sh
orochi "このIssueを実装して"
orochi -C /path/to/repository "バグを修正して"

# promptを送らずに候補と選択理由を確認
orochi --dry-run --json "このモジュールをリファクタして"

# Agentやモデルを指定する場合もPolicyとquotaの制約を適用
orochi --agent claude "..."
orochi --model '<agents --discoverで取得したID>' --reasoning high "..."

orochi status                  # 全エージェントの準備状態・CLIパス・保存済み利用状況
orochi --status                # statusの短縮形
orochi status --discover       # 実際に接続して利用可能モデルと失敗理由を確認
orochi status --json           # JSONでも取得可能
orochi sessions
orochi peers --messages        # 同じリポジトリで実行中のエージェントとメッセージ
orochi --peer-name backend "..."  # メールボックスでの名前
orochi --resume '<session ID>' "続けて入力検証を追加して"
orochi runs --limit 20
orochi policy status
```

通常のAgent回答はstdout、ルーティング理由・検証結果はstderrへ出力します。`--json`は状態確認コマンドと`--dry-run`で利用できます。タスクがサブコマンド名そのものの場合は、`orochi -- "status"`のように区切ってください。

`status`は実行履歴の有無によらず全エージェントを一覧表示します。現在のインストール・有効/無効・cooldownを反映するため、過去に成功したAgentでも削除・無効化されていればその状態を表示します。通常の`status`はAgentを起動せず、認証状態は未確認です。`status --discover`は不足アダプターを準備して接続確認を行い、接続成功時は`connected`とモデル一覧、失敗時は理由を表示します。タスクのpromptは送信せず、cooldown中のAgentは再接続を待ちます。保存済みquotaと直近の実行結果も続けて確認できます。

終了コードは`0`（完了／検証なしの完了）、`1`（実行・評価・設定エラー）、`2`（CLI引数エラー）、`130`（中断／権限拒否）です。

### 対話モード

端末でタスクを付けずに`orochi`を起動すると、対話セッションになります。`orochi chat`でも同じです。画面・操作・メッセージの流れ方は[OpenHands CLI](https://docs.openhands.dev/openhands/usage/cli/terminal)（[ソース](https://github.com/OpenHands/OpenHands-CLI)）と[Claude Code](https://code.claude.com/docs/en/interactive-mode)の慣習に合わせています。どちらも全画面のTUIですが、Orochiは端末に順に出力していく方式です。

```sh
orochi                              # 対話を開始
orochi -c                           # このディレクトリの直近の会話を続ける（--continue）
orochi --resume '<session ID>'      # 記録済みセッションの続きから対話
orochi --agent claude               # 最初のメッセージと再ルーティング時のAgent指定
orochi --yolo                       # 権限要求を毎回一度限りで許可（--always-approve、--permission allowと同じ）
```

```text
> ノートを更新して
  ⎿ codex · gpt-5.6-luna · medium            選ばれたAgent（Provider別の色）。経路が変わったときだけ表示
✻ Inspecting the fixture                    思考の要約（薄い斜体）

⏺ I'll list the files first.                Agentの回答。文字は届いた順に表示し、行が確定したら整形

⏺ List fixture files: $ ls ✓                実行中は灰色の行とスピナー。完了するとその行が緑の✓／赤の✗に変わる
  ⎿ a.txt … +1 lines                        結果の1行目と残りの行数

⏺ Edit notes: notes.txt ✓
  ⎿ Updated notes.txt with 1 addition and 1 removal
      - old line                            差分（赤／緑）
      + new line

⏺ Plan                                      計画（☒ 完了、☐ 実行中は強調）
  ⎿ ☒ Inspect files
    ☐ Report back

✻ 12s · codex · gpt-5.6-luna · ↑ 10.2k ↓ 1.1k · cache 40%
```

- **表示は常に上へ流れ続けます。** 入力欄が複数行に伸び縮みしても、実行中に次のメッセージを打っても、前の出力を描き直したり上書きしたりしません（擬似端末で画面を再現するテストで固定しています）。
- **入力欄は常に画面下部に固定します。** 履歴はその上を流れ、端末のスクロールバックにも残ります。入力欄の下の状態行に、承認モード・順番待ちの件数・作業状況（`⠹ Working (12s) · esc to interrupt`）を表示します。
- **実行中もそのまま入力できます。** Enterで送ると順番待ちに入り（`⏸ queued (1): …`）、実行中の作業が終わり次第、順に処理します。
- **Shift+Tabで承認モードを切り替えます。** エージェントがACPで提供するセッションモード（Claude Agent ACPなら`default`／`acceptEdits`／`plan`など、Codex ACPなら`read-only`／`auto`など）があればそれを巡回し、無ければOrochi側の`ask`／`always`／`never`を巡回します。`/confirm`でも変更できます。
- **ファイルや画像を添付できます。** パスを貼り付ける（ドラッグ＆ドロップ）か、`@`のあとにパスを書いてTabで補完すると、入力欄に`[Image #1]`／`[File #1]`のタグが入り、その下に一覧が出ます。画像はACPの`image`ブロック、テキストは`resource`ブロックとして送ります。エージェントが対応していない場合や8 MiBを超える場合は、ファイルの場所（`resource_link`）として送ります。
- **段取りはOrochiが決めます。** メッセージごとに分類し、小さい依頼はそのまま1回で実行、設計変更を伴うものや大規模なものは「設計 → 実装（→ レビュー）」に分けて、段ごとに別々のAgent・モデルを選び直します。設計は推論の強いモデル、実装は速いモデル、という使い分けになります。前の段の回答は次の段へ引き継ぎ、追加の指示は実装を担当したセッションに続きます。判断は`⎿ complex task · design → implement`のように表示します。
  - 実装が正常に終わらなかった場合はレビューを追加し、レビューが`VERDICT: fix`と答えた場合だけ、もう1回だけ修正の段を追加します。
  - `/team <タスク>`で3段を強制、`/solo <タスク>`で1回だけの実行を強制できます。
- **人数を頼めば、その人数を同じプロセス内で起動します。** 「5人くらいのエージェントでディスカッションして」のように頼むと、Orochiが**そのぶんのAgentセッションを1つのプロセス内で同時に起動**します（最大6席）。席は`facilitator`（進行役）＋`skeptic`／`architect`／`simplifier`／`operator`／`advocate`のように**違う視点**を割り当てます。議論だけの依頼では**全員が読み取り専用**になり、リポジトリは変更しません。エージェントに`orochi`コマンドを実行させて別プロセスを立ち上げさせることはしません（プロンプトでも明示的に禁止しています）。
- **大きい依頼には、もう1つの席を横に置きます。** 設計変更を伴う・規模が大きい・長丁場と判断された依頼では、Orochiは作業役の横に**同じプロセス内でもう1つのAgentセッションを同時に**起動します。席の名前と役割はタスクから決めます（構造を変える依頼なら`architect`、曖昧な依頼なら`researcher`、それ以外は`reviewer`。作業役も`implementer`／`fixer`／`migrator`のように決まります）。2つ目の席は**読むだけ**で、ファイルを変更するツールはOrochi側で拒否するため、作業ツリーは1つのままで競合しません。2者はメールボックスで会話し（Orochiが足す指示文は英語ですが、**回答もエージェント同士のメッセージも依頼と同じ言語で書く**よう指示します）、その内容はそのままチャット表示に流れます（`⎿ reviewer test · astra-test`のように、どのAgent・モデルが座っているかも表示します）。**席ごとに違うAgent・モデルを割り当てます。** 他の席が使っている(Agent, モデル)の組み合わせは候補から外し（他に選択肢がない場合だけ同じものを使います）、同じモデルが並ぶのを避けます。作業役が終わると2つ目の席も終了します。「エージェント同士で相談しながらやって」のように**依頼そのものが協働を求めている場合**は、規模にかかわらず2席にします。**席は毎回そのターン限り**なので、議論の続きを頼むと同じ人数で座り直します（前のターンの内容は引き継ぎます）。`/solo <タスク>`なら1人に固定します。
- **会話が手に負えなくなったら、選び直します。** 会話は同じAgent・モデルで続きますが、依頼が重くなって**そのモデルなら最初から選ばれなかった**水準（`scheduler.required_success`に届かない）になった場合、そのターンだけ固定を外して選び直します（`⎿ this needs more than the model in this conversation · picking again`）。設計変更や大規模な依頼は段に分かれるので、そこでも選び直されます。
- **利用枠の都合で強いモデルを避けたときは、そう言います。** 残量が少ないモデルは選定コストが上がるため、本来の1番手を譲ることがあります。その場合は`⎿ fable is close to its limit here; using sonnet instead`のように1行残します。
- **足りない能力は、できるAgentへ渡します。** 会話の途中で画像生成（`image`）、ブラウザ操作（`browser`）、Web検索（`web`）が必要になった場合、担当中のAgentが対応していなければ、その依頼だけ対応できるAgentが実行し、会話自体は元のAgentに戻ります（結果は次の依頼で元のAgentに伝えます）。能力は`[[agents]]`の`image` / `browser` / `web`で宣言します。`--agent`で指定している場合も、能力が足りないときはこの引き渡しが優先されます。
- **同じリポジトリで動いているAgent同士で、リソースを譲り合います。** 他の参加者（別プロセスのOrochiだけでなく、同じターンの別の席も含む）が使用中のAgent（アカウント）は、選定時のコストを1.4倍にして優先度を下げます。ほかに選択肢がなければそのまま使います。エージェントへの指示にも「同じファイル・アカウント・ツールが必要なときは、先に相談して順番を決める」よう明記しています。
- 同じリポジトリで動いている他のOrochi（別プロセス・別worktree）とのやり取りを、チャット形式で表示します。**参加者はAgentセッション単位**なので、1つのOrochiが動かしている複数のAgent同士（`/team`の各段や`collaborate`の各担当）も、互いに認識してメッセージを送れます。参加者として登録するのは実際に作業を始めたセッションだけで、モデル一覧の取得のために開いたセッションは登録しません。
- 各参加者が**どのAgent・どのモデルを使っているか**を表示します。メッセージの見出し（`✉ backend (codex · gpt-5.6-luna) → frontend`）、`/peers`の一覧、`list_peers`の結果（`agent`／`model`）で確認できます。

```text
● backend joined · ~/dev/app-worktree · feature/api · codex / gpt-5.6-luna
✉ frontend (this session) → backend  12:03
  ▎ The list endpoint now returns 201 for creates.
✉ backend → frontend (this session)  12:03
  ▎ Understood, I will update the client.
○ backend left
```

  相手ごとに色を変え、自分のセッションはブランド色で示します。参加・離脱も表示します。入力中に届いたメッセージは、入力行を壊さないように、送信した直後にまとめて表示します。自分のAgentがメールボックスのツールを使ったときは、ツール行の代わりに「Messaging another agent」などの状態表示にします。`/peers`で現在動いているAgentの一覧を確認できます。`[mailbox] enabled = false`の場合は表示しません。
- 色は役割ごとに固定しています。ロゴ・プロンプト・スピナー・計画の実行中項目はブランド色、Agentの回答は青、成功は緑、失敗は赤、注意・権限要求は黄、補足情報はグレーです。Markdownの見出し・太字・`コード`・コードブロック・箇条書きも色分けします。`NO_COLOR`を設定するか、端末以外へ出力する場合は色を付けません。
- **権限の既定は自動承認（auto）です。** 対話モードは既定で、Agentからの権限要求に一度限りの許可を自動で返します。止めて確認させたい場合は`--permission ask`か`/confirm ask`、すべて拒否するなら`/confirm never`です（設定の`scheduler.permission`で`allow`／`deny`を明示した場合はそちらを使います。単発実行`orochi "タスク"`の既定は従来どおり`ask`です）。
- 確認が必要な場合は、**入力欄がそのまま確認画面に置き換わります**。`1 Yes`／`2 No`／`3 Auto`を数字キーか↑↓とEnterで選び、Escで拒否します。答えると入力欄に戻り、履歴には`⏺ <ツール名> → allowed once`の1行だけが残ります。
- **Orochi自身のメールボックスのツール（`orochi-mailbox`）は確認しません。** 他のAgentと連絡するだけで、ファイルもコマンドも触らないためです。やり取りはチャット表示に出ます。他に動いているAgentがいない場合、`read_messages`は待たずにすぐ返します。
- 拒否すると、そのメッセージの作業は止まります。次のメッセージで別のやり方を指示してください。
- 未インストールのAgentは表示しません。認証失敗など、実際に使えないAgentだけをセッション中に1回警告します。

通常実行との違い:

- 入力はそのまま送ります。通常実行の「コーディング作業として検証する」前置きは付けません。
- 最初のメッセージでAgent・モデルを選び、2通目以降はそのセッションを`session/load`で読み込んで続けます。固定バージョンのCodex ACP 1.10.0とClaude Agent ACP 0.77.0は、ソース上で`loadSession`を広告しています。`session/load`に対応しないAgentでは、同じAgent・モデルの新しいセッションに、それまでの会話（メモリ上に最大約32 KiB）を文脈として渡します。
- チェック（評価）は、そのメッセージの間にファイルが変わった場合だけ実行します。gitリポジトリではgitが認識するファイル、それ以外では`.git`・`node_modules`・`target`を除くファイルのサイズと更新時刻で判定します。変更がなかったメッセージは未検証（`partial_success`）として記録し、学習には使いません。
- Agentが回答を終えた後にチェックが失敗しても、別のAgentへ自動では引き継ぎません。結果を表示して入力待ちに戻ります。利用制限など、回答の途中でのエラーは通常どおり別の候補へ交代します。
- 会話の継続中はAgent・モデルを固定するため、利用制限などが起きても別のAgentへ自動では交代しません。`/reroute`で、それまでの会話を文脈として渡したまま次のメッセージを再ルーティングできます。
- リポジトリのロックは、メッセージの実行中だけ保持します。
- 入力履歴と会話はメモリ上だけに保持し、ディスクへは保存しません。SQLiteへは、通常実行と同様に本文を含まない実行記録とセッションIDだけを保存します。

| 入力 | 動作 |
|---|---|
| `/help` | コマンドとショートカットの一覧 |
| `/new`（`/clear`） | 会話を破棄し、次のメッセージを新規にルーティング |
| `/resume`（`/history`） | このディレクトリの最近の会話を一覧表示。`/resume <番号またはID>`で再開 |
| `/reroute` | 会話を文脈として渡したまま、次のメッセージを再ルーティング |
| `/confirm`（`/permissions`） | 権限要求への応答方針（ask／always／never）を表示・変更 |
| `/team <task>` | 設計→実装→レビューを、段ごとに別々のAgentを選んで順に実行（`/collaborate`も同じ） |
| `/peers` | 同じリポジトリで動いている他のAgentと、その作業ディレクトリ・ブランチ・経路・状態 |
| `/status` | 作業ディレクトリ、継続中のAgent・モデル・セッション、応答方針 |
| `/exit`（`/quit`）、Ctrl-D | 終了 |
| `/`の入力中、Tab | コマンドを補完。候補が複数あれば一覧表示 |
| `@`の入力中、Tab | ファイル名を補完して添付 |
| Shift+Tab | 承認モードの切り替え（auto → ask → never） |
| ↑↓ | このセッションの入力履歴 |
| 行末の`\`+Enter | 改行して入力を続ける。複数行の貼り付けは1つのメッセージになる |
| 実行中のEsc | その作業を中断して入力へ戻る |
| Ctrl-C | 入力内容を消す。空の状態で2回押すと終了 |
| Ctrl-D | 終了 |

実行中に打ったキーは画面に表示せず、Escと権限パネルの操作以外は破棄します。stdinが端末でない場合、タスクなしの`orochi`は従来どおりヘルプを表示します。`orochi chat`は標準入力の各行を1メッセージとして処理します。この場合は装飾・思考・実行中の行を出さずに出力し、権限要求は拒否し、入力行を権限の回答には使いません。

行編集と画面制御は自前で実装しています（`src/chat/term.rs`）。端末のスクロール領域（DECSTBM）で下部の行を固定し、入力はraw modeでキーを直接読みます。全画面（代替画面）には切り替えないため、履歴は端末のスクロールバックに残ります。終了時はスクロール領域・bracketed paste・termiosを元に戻します。日本語入力の確定のように複数文字が一度に届く場合も取りこぼしません。

モックACP fixtureと疑似端末（pty）で、下部固定の入力欄・実行中の入力と順番待ち・Shift+Tabの切り替え・画像とファイルの添付・逐次表示と整形・ツール行の更新・差分・計画・権限パネルの矢印キー操作・エージェント間メッセージの表示（実行中と待機中の両方）・`/peers`・セッションの継続・`/reroute`・`/new`・Escによる中断・Tab補完・日本語入力・狭い端末を確認しました。実機のClaude／Codexでは、変更前の版で1回対話し、チェックの失敗から別Agentへ引き継がれる問題を確認しました。変更後の版の実機確認はまだ行っていません。OpenHandsのコマンドパレット、会話履歴・計画のサイドパネル、出力の折りたたみ、Claude Codeの入力枠・下部ステータス行は、順に出力していく方式では実装していません。

### 権限

デフォルトは`ask`です。ACPの`session/request_permission`を表示し、許可は`allow_once`を選びます。TTYがない場合は拒否します。

```sh
orochi --permission allow "..."  # ACPの一度限りの許可要求に自動応答
orochi --permission deny "..."
```

OrochiはOS sandboxではありません。Agent自身が備えるツール、認証、sandbox、承認設定も適用されます。AgentがACPへ許可を問い合わせない操作は、Orochiでは仲介できません。OrochiはClient側のfilesystem/terminal capabilitiesを広告せず、Agent側のネイティブツールを利用します。

### 設定

既定の設定ファイルは`$XDG_CONFIG_HOME/orochi/config.toml`、未設定時は`~/.config/orochi/config.toml`です。

```sh
orochi config path
orochi config init
orochi config show
orochi --config /path/to/config.toml --data-dir /path/to/data "..."
```

`OROCHI_CONFIG` / `OROCHI_DATA_DIR`でも変更できます。リポジトリ内の設定を自動で実行設定として読み込むことはありません。サンプルは[examples/config.toml](examples/config.toml)にあります。

```toml
[[agents]]
id = "codex"
provider = "openai"
command = "codex-acp"
args = []
enabled = true
browser = false
web = false

[scheduler]
required_success = 0.7
routing_confidence = 0.8
max_attempts = 3
discovery_timeout_secs = 30
prompt_timeout_secs = 1800
permission = "ask"
```

`agents`を指定すると、既定の4件を置き換えます。独自IDを複数登録できます。AgentがブラウザやWeb検索を利用できる場合、`browser` / `web`を明示します。ACPの標準capabilitiesだけからこの2項目は推測しません。

### 判定役（Router）

追加設定なしではローカルの決定的ルーティングで動作します。候補の選択を別のエージェントに相談する場合は、`[[agents]]`のIDとモデルを指定します。判定役はACPで接続し、認証は各CLIに従います。

```toml
[router]
agent = "codex"
model = "gpt-5.6-luna"   # 省略するとPolicyで判定向きのモデルを自動選択
session_overhead_tokens = 24000
```

低confidenceかつ候補が複数ある場合だけ相談します。新しい一時ディレクトリのセッションで、permissionを拒否して実行します。タスク本文・ファイル名・リポジトリパスは送らず、分類属性と上位12候補のみを送ります。未知の候補ID、失敗応答、過大な回答は採用しません。推薦が最良候補の期待コストの1.25倍を超える場合も採用しません。判定役だけに使うエージェントは`routing_only = true`にします。Frontier Judge・Councilと設定項目の詳細は[適応ルーティング](docs/adaptive-routing.md#4-routerfrontier-judgecouncil)を参照してください。

### 評価

デフォルトの自動評価は次のとおりです。

- Rust: `cargo test --quiet`
- Go: `go test ./...`
- Node.js: 存在する`test` / `typecheck` / `lint` / `build` scriptsを、lockfileに合うpackage managerで実行
- Git repository: staged / unstaged双方の`git diff --check`

`CI=true`、stdinなしで実行します。明示したchecksは自動検出を置き換えます。Python等は次のように設定してください。

```toml
[evaluator]
auto = false
timeout_secs = 300

[[evaluator.checks]]
name = "tests"
command = "python3"
args = ["-m", "pytest", "-q"]
```

実行コマンドはshell文字列へ展開せず、実行ファイルと引数を個別に渡します。`--no-eval`で評価を省略できます。Agentの`end_turn`だけでは成功ラベルにしません。検証なし／diff検証のみの場合は`partial_success`、実質的なチェック通過を`success`、失敗・未完了を`failure`として記録します。チェック通過は要件の意味的な完全達成を保証するものではありません。

## Policyとコストモデル

[policies/](policies/)のJSONを同梱しています。公式資料に基づくreasoning方針と、Orochi固有の性能推定を分離して扱います。

**`success_prior`・`relative_tokens`・cache割引率は、ベンチマーク未校正のOrochiヒューリスティックです。Providerが公表した成功率や料金ではありません。** パターンはACPで取得したモデルIDへのprior適用だけに用い、モデルIDを生成しません。

概略のスコアは、`(期待tokens × cache補正 + context復元) × quota係数 + latency`を推定成功確率で割ったものです。初期priorの重みを16として、context別EWMAでローカル評価履歴と混合します。成功確率の下限未満、hard constraint違反、cooldown中の候補は除外します。任意設定のBandit探索もこれらの制約を通過した候補に限定します。`orochi calibrate`で予測誤差、`orochi benchmark --input ...`で測定済み候補の比較を確認できます。

```sh
orochi policy update                          # 同梱版をインストール
orochi policy update --from ./policies        # 3つのJSONを読み込み
orochi policy update --from ./registry.json   # registry全体を読み込み
orochi policy update --url https://example.org/registry.json --sha256 '<64桁のdigest>'
```

registry形式は`{"schema_version":1,"policies":[...]}`です。全件を検証してからアトミックに置換し、検証失敗時は既存版を保持します。リモート更新はHTTPSと明示digestを要求します。公式資料の自動スクレイピング、署名付き配布サービス、更新先サーバー自体は実装していません。更新元の選定は利用者が行います。

Policyの出典:

- [OpenAI model guidance](https://developers.openai.com/api/docs/guides/latest-model)
- [Anthropic thinking / effort](https://platform.claude.com/docs/en/build-with-claude/thinking-steering-and-cost)
- [Google thinking](https://ai.google.dev/gemini-api/docs/thinking)、[context caching](https://ai.google.dev/gemini-api/docs/caching)

## Quota・usage・telemetry

データは`$XDG_DATA_HOME/orochi`、未設定時は`~/.local/share/orochi`へ保存します。

- `telemetry.sqlite3`: runs、sessions、runtime、quota_snapshots、スキーマversionとローカルsalt（今回schema v2へ移行）
- `policies.json`: インストール済みPolicy
- `locks/`: 同一リポジトリ実行のロック
- `adapters/`: 自動導入したACPアダプターとnpmキャッシュ

タスク本文、会話、ソースコード、diff本文、Agent stderr、テスト出力はDBへ保存しません。repository識別子はローカルsaltを使ったハッシュです。TaskEnvelopeは実行中のメモリにのみ保持します。Agent自身が保存する会話やProvider側の記録は別です。

一般的な認証／rate-limitエラー、構造化された`resetAt` / `reset_at` / `resetsAt`（Unix秒）、`retryAfter` / `retry_after`（秒）を観測します。`data.scope = "model"`が明示されなければrate-limitはAgent全体に適用します。標準化されていないサブスクリプション残量を推測してhealthyと表示することはありません。

アダプターが残量を提供できる場合は、応答またはsession updateで次の拡張を使用できます。

```json
{"_meta":{"orochi.dev/quota":{"remaining":0.08,"reset_at":2000000000,"model":null}}}
```

`remaining`は0〜1、`model: null`はAgent全体です。Provider内部の非公開quota APIは呼びません。`orochi quota --refresh`でCodex公式CLIの残量を直接取得できます。Claudeは公式`/usage`の随時取得（Python 3 / POSIX）と`quota-ingest`による公式statusline入力に対応します。Antigravityも`agy`の公式`/usage`取得を実装していますが、認証済みの表示パースは未検証です。[取得元・期限・制約](docs/adaptive-routing.md#3-残量)を参照してください。

usageは応答のdraft `usage`（camelCase）または`_meta["orochi.dev/usage"]`（turn単位）から取得します。OpenAI互換とAnthropic形式を正規化し、reasoning/cacheを二重加算しません。欠落値は`null`であり、ゼロでも推定tokensでもありません。draftのusage semanticsやアダプターの報告精度に依存するため、実測値の厳密な比較にはアダプター側の確認が必要です。

cache affinityは同じrepository・設定・指示ファイル・Agent・モデル・reasoning・modeの直近セッションから推定します。ACPでcache keyやadaptive thinking設定が公開されていなければ、Provider固有パラメーターを勝手に送信せずAgent側に任せます。

## 開発・検証

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

E2EテストはPython 3のACP fixtureを実際の子プロセスとして起動します。判定役もACP fixtureで検証します。実アカウント・外部LLM・API課金は不要です。CIはLinux/macOSを対象にします。Windowsの実機検証と子孫プロセス一括終了は未対応です。

[examples/demo.py](examples/demo.py)で、アカウントなしの一連の動作を試せます。

```sh
cargo build --locked
python3 examples/demo.py
```

テストはACP contractに対する検証です。各実Providerのログイン済み環境での互換性は、使用するアダプターversionとともに別途確認してください。

[CLI自動検出の実機E2E結果](docs/cli-discovery-e2e.md): Claude/Codex両方の候補登録と、Claudeへの実行委譲を確認済みです。

## 構成

```text
src/acp.rs          ACP接続・session・動的設定・stream・permission
src/discovery.rs    CLI検出・ACP接続方式の解決・不足アダプターの準備
src/agents.rs       Providerごとのerror / quota / usage正規化
src/router/         Task Profiler・ACPによるタスク分類、候補生成・スコア、ACP判定役によるRouter・Judge・Council
src/scheduler/      実行・fallback・quota / circuit breaker
src/process.rs      Agentの監視プロセス・子孫の記録・強制終了後の回収
src/mailbox.rs      プロセス間のエージェント間メッセージ（MCPサーバー・期限付き保存）
src/memory.rs       セッションを跨ぐ記憶（ユーザーの好み・リポジトリの覚え書き。テレメトリとは別保存）
src/context.rs      TaskEnvelope、Git情報、cache key
src/evaluator.rs    ローカル評価とプロセス管理
src/storage.rs      SQLite・schema・repository lock
src/learning.rs     EWMA・Bandit・予測校正
src/benchmark.rs    測定済み候補の時系列比較・holdout係数探索
src/collaboration/  独立ACPセッションの工程・タスクからのチーム編成・並列実装のマージ・メッセージ・作業ツリーへの適用
src/quota_terminal.py ネイティブCLIの読み取り専用残量取得
src/quota_sources.rs CLI残量取得・statusline取り込み
src/gateway.rs      OrochiのACP v1 stdio公開
src/chat/           対話モード（画面表示・順番待ち・承認モード・添付・セッション継続）
src/chat/term.rs    端末制御（下部固定の入力欄・キー入力・スクロール領域）
src/policy.rs       Policy検証・更新
src/config.rs       設定・既定のAgentプリセット
src/cli.rs          ユーザー操作
```

ACP仕様の参照: [Rust SDK](https://github.com/agentclientprotocol/rust-sdk)、[Session Config Options](https://agentclientprotocol.com/protocol/v1/session-config-options)。

## License

MIT
