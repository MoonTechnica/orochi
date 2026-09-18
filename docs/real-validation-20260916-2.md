# 残タスクの実機検証（2026-09-16 後半）

[前半の検証](real-validation-20260916.md)で残った7項目の対応結果。証拠は`.orochi/live-e2e/remaining-20260916/`に保存した（ローカルのみ・非追跡）。

| # | 項目 | 結果 |
|---|---|---|
| 1 | 選択精度の追加校正 | 6課題を実測。context別EWMAの限界を確認し、`pooling`を追加（既定は無効） |
| 2 | Antigravityの実機検証 | **未完了**。検証終了時点でもGoogleログイン前（`agy models`がサインインを要求） |
| 3 | 実サービスの制限による交代 | 実Codexの利用上限→実Claudeへの交代を確認。分類の不具合を修正 |
| 4 | Router／Judge／Councilの実サービス | CLIをACPで使う判定役を追加し、3方式とも実サービスで交代を確認。その後、判定役をACPに統一しHTTP endpoint版を削除（8章） |
| 5 | ZedからのACP E2E | Zed 1.19.2のエージェントパネル→Orochi→実Codex→評価まで成功。設定は元に戻した |
| 6 | 協議の拡張 | 並列実装・宛先指定の多往復通信・作業ツリーへの適用と競合解消を実装。実Claude／Codexで確認 |
| 7 | 強制終了からの復旧 | 監視プロセスと記録による回収を実装。実ClaudeでSIGKILL→回収→再開を確認 |

## 1. 選択精度の追加校正

`examples/diverse-suite.json`を追加。Python・JavaScript・Rust・ドキュメントで、難易度と規模が異なる6課題を用意した。各課題の判定スクリプトは、開始状態で失敗し参照解で成功することを事前に確認した。

Claudeの週間枠が残り約6%だったため、ユーザーの判断で候補を**Claude Haiku**と**Codex gpt-5.6-luna / medium**にした。Complexのリファクタリング課題は、両モデルの成功確率priorが`required_success`（0.7）未満で、Orochi自身が候補から除外した（期待どおりの動作）。この課題だけ、Policy上の候補になる**Claude Sonnet / high**と**Codex gpt-5.6-sol / high**で測定した。

| 課題 | Profiler | Haiku | luna / medium |
|---|---|---|---|
| 四則演算の評価器（Python） | implementation / normal | **失敗**（`"1 2"`を12として受理）・189,819 tokens | 成功・28,152 |
| 時間文字列の解析（JavaScript） | implementation / normal | 成功・86,527 | 成功・25,565 |
| 括弧の対応（Rust、cargo test） | test / normal ※ | 成功・211,159 | 成功・28,348 |
| Decimalでの金額計算の修正（Python、2ファイル） | bug_fix / normal | 成功・323,755 | 成功・29,466 |
| READMEの追記 | documentation / simple | 成功・80,429 | 成功・24,991 |

| 課題 | Profiler | Sonnet / high | sol / high |
|---|---|---|---|
| 出力を保つリファクタリング（JavaScript） | refactor / complex | 成功・219,615 tokens・16秒 | 成功・27,907 tokens・69秒 |

※ 1行目で「単体テストも追加」と求めたため、実装課題がtestに分類された。「Implement …」で始まる英語の依頼は実装として扱うよう修正し、テストを追加した。

tokensは各ACPアダプターの報告値で、料金ではない。ProviderによってcacheやシステムプロンプトのtokensのUsage報告が異なる。初期推定（約9,500 tokens）との比は、Codexが約2.7〜3倍、Claudeが約8.5〜34倍だった。

### 分かったこと

- 6課題はすべてcontext（task type・言語・framework・難易度）が異なる。context別EWMAでは課題間で学習が共有されず、Static・EWMA・Banditが同じ選択になった。
- 初期priorが同じ候補の順番は候補IDで決まる。今回はたまたまCodexが先だったため、学習なしでも最良の結果になった。
- tokensの超過率は課題よりAgent／モデルに依存する傾向が強い。

### 追加した改善と検証

`[learning] pooling`（0〜1、既定0）を追加した。同じ候補の他contextの実績から、初期推定に対するtokensの比と成功の差分を求め、今回のcontextの初期値へ反映する。`benchmark`のreplayと`benchmark-tune`の探索範囲（0 / 0.5 / 1.0）にも同じ処理を入れた。

6課題を全720通りの順番でreplayし、候補IDの並びを両方向で平均した（`analysis/order-robust-6.json`、`analysis/order_robust_replay.py`）。

| 方式 | 並び: 元のまま | 並び: 逆 | 平均 |
|---|---|---|---|
| Static / EWMA / Bandit（pooling 0） | 6.0成功・164,429 tokens | 5.0成功・1,111,304 | 5.5成功・約64万 |
| EWMA / Bandit（pooling 0.5または1.0） | 5.8成功・315,462 | 5.8成功・507,170 | **5.8成功・約41万** |

- poolingは並び順の偶然に左右されにくくなり、平均では成功数・tokensとも改善した。一方、既定の並びがすでに最良だった場合は、一度Claudeを試す分だけ悪化した。
- 実際の時系列順で前半2〜4課題から係数を選ぶと、いずれも既定値（alpha 0.1、prior_weight 16、exploration 0.1、pooling 0）のままで、後半の改善はなかった（`analysis/tune-6-train{2,3,4}.json`）。
- BanditはEWMAと同じ結果になった。学習後は他の候補が最小コストの1.25倍以内に入らず、探索が起きなかった。

**6課題・2候補の小規模データであり、一般的な改善は確立していない。既定値は変更しない。** 課題の種類が多く、tokensの傾向がAgentごとに大きく違う環境では、`pooling = 0.5`を明示して試す価値がある。

証拠: `calibration/{dataset,evidence}.json`、`calibration-complex/`、`analysis/`。

## 2. Antigravity

`agy models`は、検証の開始時と終了時のどちらでも「Please sign in」を返した。ログイン後に次を確認する予定。

```sh
.orochi/tools/antigravity/agy          # 対話形式でGoogleログイン
orochi quota --refresh                  # antigravity_usageプローブ
orochi status --discover                # モデル一覧
orochi --agent antigravity "..."        # 実行と評価
```

未認証の公式ACP Server（1.1.1）は、判定役の検証で`Authentication required`を返した。`authentication`として記録され、次の候補へ交代した（4章）。

## 3. 実サービスの制限による交代

Codexのクレジットが切れていたため、**実Codexの利用上限→実Claude**の方向で確認した。実Claudeの週間枠は残り約5%で、上限には達していない。Claude→Codex方向の実サービスでの交代は未確認のまま。

codex-acp 1.10.0の実際の応答:

```json
{"code":-32603,"message":"Internal error","data":{"message":"You've hit your usage limit. ... try again at 9:11 PM.","codexErrorInfo":"usageLimitExceeded"}}
```

1回目の実行では`Other: Internal error`と分類され、Claudeへの交代はしたものの、アカウント単位のcooldownが記録されなかった。`data.message`と`data.codexErrorInfo`を分類対象に加えた。「try again at 9:11 PM」をローカル時刻の次の9:11 PMとして解釈するようにし、実際のpayloadで回帰テストを追加した。

修正後（`failover-run2/`）:

1. `codex / gpt-5.6-luna`: `RateLimit`（1.6秒）。`codex/*`に21:11までのcooldownを保存。
2. 同じ実行内で`claude / haiku`へ交代し、チェックに合格（145,906 tokens）。
3. 同じデータで2回目を実行すると、Codexは接続前に`agent/account is in cooldown`で除外され、Claudeで完了。同じcontextのHaikuのtokens実績が学習され、今回はSonnet / mediumが選ばれた。

協議（6章）でも、司令塔の実Codexが同じ上限で失敗し、実Claudeが引き継いだ。Codexを希望した並列実装者は、cooldownによりClaudeで開始した。

## 4. Router／Judge／Council（ACP判定役）

HTTP endpoint版はOrochi自身がAPIキーで接続する。APIキーを使わない方針のため、実サービスでは未検証。代わりに`acp`設定を追加した（[設定](adaptive-routing.md#acp判定役)）。ACP判定役の認証は各CLIに従う。今回の環境にはProviderのAPIキーの環境変数がなく、Claudeは5時間・週間枠、Codexはプラン変更を促す上限表示が出ていた。どちらもサブスクリプションのログインで動作していた。実行Agentはfixtureで、判定役だけが実サービス。

| 方式 | 判定役の順番 | 結果 |
|---|---|---|
| Frontier Judge | Antigravity → Codex luna → Claude Haiku | Antigravityが`authentication`で失敗し、Codexが有効な候補IDを返した（23,316 tokens・4.5秒）。推薦は制約を通過 |
| Council（2席×2ラウンド） | 席1: Claude Haiku、席2: Antigravity → Codex luna | 第1ラウンドで席2がCodexに交代。第2ラウンドは交代先から開始。過半数の推薦が制約を通過。判定役の合計103,612 tokens |
| 軽量Router（`routing_confidence = 1.0`で強制） | Antigravity → Claude Haiku | Haikuが推薦（28,326 tokens・6.6秒） |

- 1回あたりの消費はCodex約2.3万tokens、Claude Haiku約2.8万tokens。システムプロンプト等によるもので、推定予算に`session_overhead_tokens`を加える根拠にした。
- Councilの予算検査（約15.5万tokens）を通すため、合成した長い移行仕様（約18万文字）を課題にした。課題本文は判定役へ送らず、ローカルの推定tokensを増やす目的だけに使った。
- 判定役のプロンプトにタスク本文・リポジトリのパスが含まれないこと、作業ディレクトリがリポジトリ外の一時ディレクトリで、終了後に削除されることをfixtureテストで確認している。

証拠: `advisers/`（設定・stderr・データディレクトリ）。

## 5. ZedからのACP E2E

ユーザー承認のもと、`~/.config/zed/settings.json`に`agent_servers`、`keymap.json`に`agent::NewExternalAgentThread`の一時バインドを追加した。専用プロジェクトのウィンドウを開き、System Eventsでキー入力した。

1. Zedが`orochi serve`を起動（Zedログ: `connection; name="zed"`）。
2. エージェントパネルからプロンプトを送信。
3. OrochiがCodex gpt-5.6-luna / mediumを選択し、`zed_result.txt`（`OROCHI_ZED_OK\n`）を作成。評価に合格（24,170 tokens・23.8秒）。Zedのログに、Orochiが標準エラーへ出したルーティング結果が記録された。
4. テスト用ウィンドウだけを閉じ、`serve`プロセスの終了を確認。設定2ファイルをバックアップから戻し、SHA-256の一致を確認した。

この環境では画面キャプチャが許可されていないため、画面表示は目視・画像では確認していない。確認したのは、Zedのログ、Orochiの実行記録、生成ファイル。

証拠: `zed/`（バックアップ、gateway設定、データ）。

## 6. 協議の拡張（実Claude／Codex）

`collab-live/`: 司令塔（Codex希望）、実装者2人（alice: Claude Haiku、bob: Codex希望）、レビュー者、統合者（Claude Sonnet）、協議2ラウンド、`--apply`。開始直後に、元の作業ツリーの`textstats/__init__.py`と`README.md`を手で編集した。

- 司令塔: 実Codexが利用上限→Claude Haikuへ交代。
- aliceとbobは同時に開始（26.6秒・36.3秒）。bobはCodexのcooldownによりClaudeで実行。マージの競合は0件。
- メッセージ11件（拒否0件）。司令塔→各実装者、bob→alice、レビュー者→統合者などを送信。aliceは協議ラウンド1で返信し、その後に再マージした。
- 統合者（Sonnet）はチェックに合格。
- 作業ツリーへの適用で`__init__.py`が競合した（ユーザーの`__version__`追加と、協議側のexport追加）。解消セッション（Sonnet）が両方を残し、チェックとマーカー確認に合格。3ファイルを適用した。
- 独立確認: 元の作業ツリーでチェックが通る。ユーザーのREADME編集と`__version__`が残っている。マーカーはない。11回の実行のセッションIDはすべて異なる。残存プロセス・記録はない。
- 合計1,672,438 tokens（アダプター報告値）、エージェント実行時間は約223秒。

## 7. 強制終了からの復旧（実Claude）

`kill-resume/`: 実装者（Claude Haiku）の応答中に、`orochi collaborate`をSIGKILLした。

- 記録にあったプロセスは4つ。監視プロセス、`claude-agent-acp`（node）、`claude` CLI、CLIが起動したMCPサーバー（別アプリ提供）。5秒後の確認では4つとも終了し、記録も削除されていた。
- レポートには`running`の実行と93文字の途中回答が残っていた。
- `collaborate-resume`で、その実行を`interrupted`とし、新しいセッションで同じ工程をやり直した。3工程が完了し、統合結果のチェックに合格。

監視プロセスごと終了させた場合の回収（次回起動時）、グループを離れた子孫の終了、所有者が動作中の記録に触れないことは、fixtureテストで確認している。

## 8. 追記: 判定役のACP統一

ユーザーの判断で、Router／Judge／CouncilのHTTP endpoint版を削除し、ACPに統一した。4章の`acp`設定は統一前の形式で、現在は次の形式になる。

- 判定役は`[[agents]]`のIDとモデルで指定する（`agent`・`model`・任意の`reasoning`）。判定役だけに使うエージェントは`routing_only = true`で、実行の候補にはならない。
- `endpoint`・`api_key_env`・`acp`を含む旧設定は、移行方法を示すエラーになる。既定値は`timeout_secs = 120`、`max_resource_fraction = 1.0`、`max_output_tokens = 512`（予算計算用）。
- 判定役がアカウント単位の制限・認証エラー・接続不能になった場合は、そのエージェントのcooldownに記録し、cooldown中は起動しない。記録には実際の判定役のAgent・モデルを残す。
- Councilの第2ラウンドは、第1ラウンドに答えたセッションへ票だけを送る。
- `model`は任意。省略すると、役割ごとの難しさ（Router: simple、Council: normal、Frontier Judge: complex）で、エージェントのモデルをPolicyに基づいて選ぶ。実Codexでは、Frontier Judgeが`gpt-5.6-sol` / high（24,927 tokens・5.2秒）、Routerが`gpt-5.6-luna` / low（23,332 tokens・5.5秒）を選んだ。途中の実装で一時的にCLIの既定モデルを使ったときは`gpt-6-astra`（20,830 tokens）だった。合計tokensの大部分はシステムプロンプトで、モデルによる差は小さい。tokensは料金ではなく、モデルごとの単価や利用枠の消費率の違いは含まない。

統一後の実サービス確認（`advisers-acp/`、実行Agentはfixture、判定役はすべて`routing_only`）:

| 方式 | 結果 |
|---|---|
| Frontier Judge: Antigravity → Codex luna → Claude Haiku | Antigravityが`authentication`で失敗し、`antigravity/*`にcooldownを記録。Codex lunaが推薦（23,374 tokens・5.5秒） |
| Council: 席1 Codex luna、席2 Antigravity → Codex sol（同じデータディレクトリ） | 席2のAntigravityはcooldownのため起動せずに交代（0秒）。過半数の推薦が制約を通過 |

Councilのセッション再利用（各アダプターの報告値）:

| 判定役 | 第1ラウンド（新規） | 第2ラウンド（同じセッション） |
|---|---|---|
| Codex luna | 23,387 tokens（cache 11,008）・4.9秒 | 29,323 tokens（cache 22,272、非cache入力 7,003）・2.4秒 |
| Codex sol | 24,897 tokens（cache 0）・4.4秒 | 30,806 tokens（cache 24,704、非cache入力 6,081）・2.5秒 |

再利用により所要時間はおよそ半分になり、入力の大部分がcacheになった。一方、履歴を含むため合計tokens（cache読み込みを含む）は増えた。予算検査はこの増加を含めて見積もる。

ローカルモデルは、ACP対応CLIが対応していれば実行・判定役のどちらにも使える設計だが、実機では確認していない。

## 9. 追記: エージェント間メールボックス（2026-09-17）

複数のOrochiプロセスが起動したエージェント同士のメッセージ機能を追加した。仕様と実機確認は[エージェント間メールボックス](agent-mailbox.md)を参照。同じディレクトリで同時に動かした2つの実Codexが、関数名と辞書のキーを伝え合って実装を合わせた。

## テスト

- `cargo test --locked`: Rust統合テスト103件（ACP統一・判定役のモデル自動選択・メールボックス追加後）、および内部から実行するPython残量パーサー4件が成功。
- `cargo clippy --locked --all-targets -- -D warnings`、`cargo fmt --check`が成功。
- 追加したテスト: 強制終了後の回収（監視あり／なし）、並列実装のマージと統合時のマーカー確認、宛先指定メッセージの多往復、作業ツリーへの適用・競合解消・解消中の編集の保護・未検証結果の不適用、ACP判定役のJudge・Council（`tests/router_acp.rs`へ移行。セッション再利用・cooldown記録・routing_only・旧設定の移行エラーを含む）、Codexの利用上限payloadの分類、poolingの推定値とreplay、同梱プランの検証。

## 公式資料

- [Zedの外部Agent設定](https://zed.dev/docs/ai/external-agents)
- [Codex ACP](https://github.com/agentclientprotocol/codex-acp)
- [Claude Agent ACP](https://github.com/agentclientprotocol/claude-agent-acp)
- [Antigravity CLI導入](https://www.antigravity.google/docs/cli/install)
