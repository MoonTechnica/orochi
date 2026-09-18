# 適応ルーティングとACP Gateway

実装・検証: 2026-09-15〜16。
実測の詳細は[実測と検証](real-validation-20260916.md)と[残タスクの実機検証](real-validation-20260916-2.md)、独立セッション間の通信は[セッション協議](session-collaboration.md)を参照。

## 到達点

| 項目 | 実装・検証の状態 |
|---|---|
| 選択精度の校正 | `calibrate`・`benchmark`・`benchmark-tune`を実装。Claude/Codexで同一課題8組を実測。2026-09-16に言語・難易度・規模の異なる6課題を追加実測し、context別EWMAでは異なる課題間で学習が共有されないことを確認。同じ候補の他context実績を使う`pooling`を追加（既定は無効）。小規模データのため、一般的な改善は未確立 |
| 継続学習 | context別EWMAと、制約内のepsilon-greedy Banditを実装。既定はEWMA、探索は明示設定 |
| サブスク残量 | Codex公式app-server、Claude公式statuslineと随時`/usage`を実機確認。Antigravityの`/usage`プローブは実装済み・認証後の実機確認待ち。Geminiは対象外 |
| Antigravity実機E2E | CLI・公式ACP Serverを導入済み。認証要求まで確認、Googleログイン後の検証待ち |
| Router／Frontier Judge | 判定役を`[[agents]]`のACPエージェントに統一（2026-09-16）。実Antigravity（未認証）→実Codexの交代を確認。成果物の意味的な合否判定は未実装 |
| Council / セッション協議 | 独立ACPセッションによる司令塔→並列実装→統合、宛先指定の多往復通信、元の作業ツリーへの適用と競合解消。全役割に代替Agent／モデル選択、途中状態保存、`collaborate-resume`を実装。Router／Judge／Councilは代替判定役に対応し、ACPでの2ラウンドCouncilを実サービスで確認 |
| OrochiのACP公開 | `serve`でACP v1 stdioに対応。fixtureでpermission・cancel等を検証。ACPクライアント→実Codexの実行・評価は成功。Zed 1.19.2のエージェントパネルから実Codexへの実行・評価まで確認（UI操作はSystem Eventsで自動化、画面キャプチャなし） |

## 1. 校正と比較

```sh
orochi calibrate --limit 10000
orochi benchmark --input paired-cases.json --seed 42 --failure-penalty 100000
orochi benchmark-tune --input paired-cases.json --training-cases 4

# 比較コマンドの動作確認用。実Providerの性能データではない。
python3 examples/benchmark_fixture.py > /tmp/orochi-synthetic.json
orochi benchmark --input /tmp/orochi-synthetic.json
```

`calibrate`は、その実行より前に保存された成功確率・推定tokensと実測結果を比較する。成功確率の二乗誤差（Brier score）、初期priorとの比較、10区間の予測と成功率、tokensの総絶対誤差／実測総tokens（WAPE）を出力する。低い誤差ほどよい。サンプルなし・usageなしは`null`。既存DBの予測がないrunは校正に含めない。

選ばれた候補の観測だけでは「別の候補ならもっとよかったか」は判定できない。`benchmark`には各課題を**同じ開始状態から独立に**各候補で実行した測定結果を与える。失敗も保存し、課題IDは一意にする。課題の順番を時系列として使い、各方策は自分が選んだ候補の結果だけで学習する。全候補の結果は、比較対象となる最良の実測結果の算出に使う。

入力は`schema_version: 1`、`provenance`（出典）、`synthetic`（合成データか）、`cases`のJSON。caseは`id`、`TaskDescriptor`形式の`descriptor`、`arms`を持つ。armには初期priorの`ExecutionCandidate`形式の`candidate`、評価済みの`success`、実測`tokens`、`duration_ms`を入れる。[生成例](../examples/benchmark_fixture.py)を参照。

Static / EWMA / Banditの成功数、選択・棄権数、総tokens、成功1件あたりtokens、失敗ペナルティ込みのコストと最良結果との差を出力する。失敗ペナルティは資源換算の比較パラメーターであり料金ではない。棄権にも同じペナルティを課し、基準は全候補と棄権の最小コストとする。成功数を保たずにtokensだけ減った結果を改善とはみなさない。

通常ルーティングとreplayはEWMA、探索、資源スコア関数を共有する。`candidate.prediction.cost_features`がある入力はcache/context/quota項を再利用し、ない入力は与えた初期コストから比例係数を復元する。quota/capability/Policyによる候補の適格性は入力側で確認する。replayは実Agentに接続せず、運用DBに学習結果を書き込まない。

**合成データの比較成功は実Providerでの選択精度改善や最適性の証明ではない。** まず代表的な課題を難易度・言語・規模別に用意し、独立した開始状態、固定したadapter/model/設定、同一の評価コマンドで測定する必要がある。

## 2. EWMA・Bandit

```toml
[learning]
strategy = "ewma"       # static / ewma / bandit
alpha = 0.1             # 大きいほど最近の実績に追従
prior_weight = 16.0
exploration = 0.1       # banditのみ
max_cost_ratio = 1.25
pooling = 0.0           # 0〜1。同じ候補の他context実績の反映度。既定は無効
```

Agent・model・reasoning・mode・task type・language・framework・complexityが一致する直近500runを、古いものから処理する。照合後に件数を制限する。初期priorの重みと古い観測の重みは`1-alpha`倍ずつ減衰する。tokensは実測値／その課題の初期tokens推定値を学習し、今回の課題規模に適用する。単一実行の影響を制限するため、この比率は0.05〜20に制限する。usageのないrunはtokens学習には使わない。

`pooling`を0より大きくすると、同じAgent・model・reasoning・modeの**他context**の直近500runから、初期推定に対するtokensの比と成功の差分（実測−初期prior）を求める。これを`prior_weight`で縮小し、`pooling`倍して今回のcontextの初期値へ反映する。そのcontext自身の実績は、補正後の初期値からさらに更新する。tokensの超過はシステムプロンプト・tool loop・cacheなど、課題より候補に依存する部分が大きいという実測に基づくOrochi独自の補正であり、Providerの公表値ではない。Staticでは使わない。

2026-09-16の6課題の実測では、context別EWMA（`pooling = 0`）は課題間で学習が共有されず、Staticと同じ選択になった。この6課題を720通りの順番でreplayし、同コストの候補の並び順を両方向で平均した。`pooling = 0.5`で平均成功数は5.5から5.8、平均tokensは約64万から約41万に改善した。一方、実際の時系列順では既定の並びが最良の候補を選んでいた。そのため、前半で係数を選び後半で検証しても、改善はなかった。データが小さいため既定値は変えない。詳細は[残タスクの実機検証](real-validation-20260916-2.md)。

未検証の完了、キャンセル、認証・rate-limit・接続不能・設定エラー、実行中にmodel/reasoning/modeが切り替わったrunは品質学習から除外する。Router/Judge/Councilのusageも別purposeに記録し、実装品質の学習に混ぜない。旧runの`complexity`が不明な場合はcontext別学習から除外する。

Banditはcontextごとに推定した候補のうち、成功確率の基準を通過し、最小期待コストの`max_cost_ratio`倍以内の候補を対象に探索する。既定の探索率は10%。同じ候補が最良である場合も探索の一部に含むため、最良候補の確率は`1-epsilon+epsilon/N`、他候補は`epsilon/N`。選択確率を実行前の予測と一緒に保存する。LLMの推薦を採用した場合の選択確率は算出できないため`null`。

これはcontextを区分したepsilon-greedyであり、特徴量を共有するLinUCBやThompson Samplingではない。成功確率の下限は推定値に対する制約であり、実際の成功の保証や信頼区間ではない。下限未満になった候補は探索対象にもならない。confidence・cache割引など残る係数も今後の実測校正対象。

### 弱いラベル

実装: 2026-09-18（P3）。**重みは未較正のOrochiヒューリスティック。実利用での効果は未検証。**

学習ラベルは自動チェックが通ったか落ちたかでしか生まれないので、設計・議論・調査、テストの無いリポジトリでは何回使っても学習が進まなかった。コンソールでは、答えの直後にユーザーがしたことを弱いラベルとして記録する。新しい操作は求めない。

| 直後の行動 | ラベル | 重み |
|---|---|---|
| 答えを見てから次のメッセージを送った | 成功 | 0.2 |
| `/reroute`（答えの後でも、Escで止めた後でも） | 失敗 | 0.3 |
| Escで止めて、同じエージェントのまま次を送った | なし | — |
| `/new`、終了、答えを見る前にキューしたメッセージ | なし | — |

「失敗」はタスクが失敗したという意味ではなく、「この種類の仕事にこのエージェント×モデルは合わなかったかもしれない」という弱い証拠。Sonnetの初期値0.88で試算すると、1回で0.863、5回で0.798まで下がる（検証済みの失敗なら5回で0.614）。

**Escだけではラベルにしない**（2026-09-19変更）。止める理由は、エージェントが的外れなこともあれば、ユーザーが言い忘れに気づいただけのこともあり、止めた時点では区別できない。その後に`/reroute`したら前者、同じエージェントのまま補足を送ったら後者とみなす。

- **検証済みの結果は上書きしない。** 弱いラベルが効くのは未検証の完了と中断だけ
- 記録は実行記録への追記で、最初の1回だけ。実行前に凍結した予測には触れない
- 重みは減衰に`decay^w`で効かせる。重み1なら従来と同じ計算。確信度は検証済みだけを数える
- `orochi calibrate`は、従来の数値（検証済みのみ）とは別に`weak`としてシグナルごとの件数とBrierを出す。弱いラベルから学ぶことが効いているかは、時間を追って**検証済みの**Brierが下がるかで見る

### tier pooling（既定で無効）

実装: 2026-09-18。**合成データのreplayでのみ確認。既定では無効。**

学習はモデルIDの完全一致で引くので、モデルが更新されると実績がすべて失われ、新しいIDは静的な事前分布からやり直しになる。`learning.tier_pooling`を0より大きくすると、実績の無いモデルの出発点を、**同じprovider・同じtier・同じタスク種別・同じ複雑度・同じreasoningの、他のモデルIDの実績**でずらす。

- tierはpolicyの部分文字列パターン（`opus` → frontier）から読み出し時に求めるので、既存の記録もそのまま使える
- `unknown` tierは束ねない（どのパターンにも当たらなかったIDの寄せ集めなので）
- 動かすのは出発点だけで、そのモデル自身の実績が溜まるほど寄与は減る。既存の`pooling`（同じ候補の別文脈）と証拠が重ならないので併用できる

合成データでの確認（`tests/adaptive.rs`、`tier_pooling_carries_evidence_across_a_model_replacement`）: frontierモデルを途中で新IDに差し替え、新IDの事前分布を成功率の床より下に置くと、無効では差し替え後の30件すべてで棄権し、有効では棄権0件・60件中58件成功。**実データでの効果は未確認**なので既定は無効のまま。`benchmark-tune`の探索対象に`tier_pooling`（0 / 0.5 / 1）を加えたので、手元の測定データで効くかを確かめてから有効にする。

## 3. 残量

```sh
orochi quota --refresh
orochi quota                  # 保存されたsnapshotとstaleフラグ
orochi quota-ingest --agent claude < claude-statusline.json
```

```toml
[quota]
refresh_before_run = true     # 既定false。dry-run時も取得する
max_age_secs = 300
timeout_secs = 30
```

### Codex

既知の`codex` Agentのnative CLIが検出できた場合、`quota --refresh`は公式`codex app-server`の`account/rateLimits/read`を呼ぶ。promptは送らない。CLI自身の認証を利用する。

複数windowを保存し、期限内のものだけを適用する。`rateLimitsByLimitId`を優先し、`codex`以外のbucketはmodelへの対応が不明なため表示だけに留める。`usedPercent`が欠けているwindowを残量100%と解釈しない。quota情報が期限切れになるとルーティングへの適用を止める。実行で観測した別のcooldownは維持する。

2026-09-15、ログイン済みの実Codex CLIからprimary / secondaryの残量・reset時刻の取得を確認した。証跡は`.orochi/live-e2e/adaptive/quota-live.json`。取得後に残量は変動するため、このファイルは現在値ではない。

### Claude

`quota-ingest`はClaude Codeがstatuslineに渡す`rate_limits.five_hour` / `seven_day` / `spend_limit`を取り込む。これはCLIのイベント観測であり、任意の時点で残量を再取得するAPIではない。ユーザーが設定するstatuslineスクリプトから入力を渡す。既存のClaude設定ファイルは変更しない。入力の会話・path・その他のフィールドは保存しない。context windowの残量をサブスク残量と取り違えない。

この形式は対応するClaude Codeバージョンとアカウントでのみ提供される。欠落時は不明として扱う。Claude Code 2.1.272の実statuslineを取り込み、別の検証DBへの保存も確認済み。

随時取得は`claude_usage`プローブで公式CLIの`/usage`をPOSIX PTY越しに読み取る。Python 3が必要。標準ClaudeプリセットではネイティブCLIから自動検出し、`quota --refresh`で実取得・保存できることを確認した。専用の空の一時ディレクトリで起動し、raw画面やアカウント名を保存しない。

Antigravityには同様の`antigravity_usage`プローブを実装。未認証時はOAuthを起動せずunknownとする。実アカウントの残量表示との照合はログイン後に必要。

### その他・独自取得コマンド

```toml
[[quota.probes]]
agent = "gemini"
kind = "command"
command = "/absolute/path/to/your-quota-reader"
args = []
```

コマンドは次のJSONをstdoutに返す。shell展開は行わず、一時ディレクトリで実行する。1 MiBの出力上限とtimeoutを適用し、stderrや返却JSON全文は保存しない。失敗時は期限内の以前のsnapshotを維持する。

```json
{"schema_version":1,"windows":[{"bucket":"daily","remaining":0.35,"reset_at":2000000000,"model":null,"affects_routing":true}]}
```

`model:null`はAgent全体。modelを指定する場合はACPから取得した実際のIDを使う。Gemini専用プローブは未実装で、今回の追加対応の対象外。非公開APIの呼び出しや認証token抽出は行わない。

出典: [Codex app-server](https://github.com/openai/codex/blob/main/codex-rs/app-server/README.md)、[Claude公式statusline](https://code.claude.com/docs/en/statusline)、[Antigravityの対話式usage表示](https://www.antigravity.google/docs/cli/commands/usage)。

## 4. Router・Frontier Judge・Council

判定役は、実行前に「どの候補（Agent × モデル × reasoning）で実行するか」を選ぶ補助。タスク自体は実行しない。すべて`[[agents]]`のエージェントをACPで使う。2026-09-16まであったOpenAI互換HTTP endpoint版は、ACPへの統一により削除した（旧設定の`endpoint`・`api_key_env`は移行案内付きのエラーになる）。

```toml
# 判定役だけに使うエージェント。実行候補にはならない
[[agents]]
id = "judge-claude"
provider = "anthropic"
command = "claude"
routing_only = true

# 低confidenceの課題で候補の順番を相談する
[router]
agent = "codex"                   # [[agents]]のID
model = "gpt-5.6-luna"            # 任意。ACPで取得できるモデルID
reasoning = "low"                 # 任意
session_overhead_tokens = 24000   # 実測値に合わせる。省略時は20000

# Extreme、またはambiguity 0.7以上の課題
[frontier]
agent = "codex"
model = "gpt-5.6-sol"

# 上の判定役が使えない場合の代替（最大3件、ネスト不可）
[[frontier.fallbacks]]
agent = "judge-claude"
model = "haiku"
session_overhead_tokens = 29000
```

| 項目 | 既定値 | 内容 |
|---|---|---|
| `model` | 自動 | 省略すると、そのエージェントが提供するモデルからOrochiが選ぶ（下記） |
| `timeout_secs` | 120 | CLIの起動を含む1回の上限 |
| `max_resource_fraction` | 1.0 | 判定に使うtokensの上限。最小期待コスト候補の推定tokensに対する割合 |
| `max_output_tokens` | 512 | 予算計算に使う回答の想定サイズ。ACPでは途中で打ち切れない |
| `session_overhead_tokens` | 20000 | システムプロンプト等による1ターンあたりの追加分 |

- `agent`は有効な`[[agents]]`のIDでなければならない。`routing_only = true`のエージェントは実行のdiscovery・候補・`--agent`・`collaborate`の対象にならず、判定役だけに使われる。
- 認証は各CLIに従う（サブスクリプションのログイン、またはCLIが対応するAPIキー）。Orochi自身は認証情報を読まない。Orochiの環境変数と`[agents.env]`はCLIへ引き継ぐ。
- 毎回、空の一時ディレクトリで新しいセッションを開始し、permissionは常に拒否する。送る内容は課題属性・最大12件の候補要約・peer votesだけ。タスク本文・ファイル名・リポジトリのパスは送らない。ただし、許可を求めずに作業ディレクトリ外を読めるCLIもあるため、ツールを使わないことは指示であり、OSレベルの保証ではない。
- 例外は`[classifier]`のみ（次節）。分類だけはタスク本文なしには成立しないため、明示的にopt-inした場合に限りタスク本文を送る。

## 4.1. Classifier（タスク分類）

`router::profiler`のタスク種別・複雑度の判定はキーワードマッチで、語彙から外れた言い回しは`implementation` / `Normal`に落ちる。「CIが赤いので直して」は`bug_fix`にならず、「全部TypeScriptに書き換えて」は`Extreme`にならない。これをACPエージェントの分類で上書きする。

**既定で有効。設定は不要。** `agent`を書かなければ、有効な（`routing_only`でない）エージェントを設定順に、合計で1エージェント分の時間内で試す。実行に使うエージェントに送るということは、どのみちタスク本文が行く先に送るということなので、既定の送り先をそこに限っている。

```toml
[classifier]
enabled = false        # 止める
agent = "claude"       # 送り先を固定する
model = "haiku"        # 省略時はSimple難易度としてpolicyから自動選択（＝最安の候補）
timeout_secs = 60
```

**選ぶ余地がないものはエージェントを起動しない。** `--dry-run`、読み取り専用の席、`--agent`と`--model`の両方が指定されたルートは、ヒューリスティックの判定のまま進む。

**タスク本文を送る唯一の経路。** 送り先は他のAgentと同じくローカルで起動したCLIであり、認証はそのCLI自身のもの。Orochiは本文をどこにも保存しない。キャッシュのキーはタスク本文のsalted hashで、DBに残るのは分類ラベルだけ（`tests/core.rs`の`classifier_sees_the_task_but_caches_only_a_hash_of_it`が、DBファイルの生バイト列に本文が現れないことを確認する）。

マージは**一方向のみ**。

| 項目 | 挙動 |
|---|---|
| `task_type` | 分類結果で置換。`classifier::TASK_TYPES`の11種以外は破棄（未知のラベルは学習の層を割ってしまうため） |
| `complexity` | `max(heuristic, classifier)`。**下げない** |
| `requires_*` / `long_horizon` / `requires_architecture_change` | 論理和。heuristicが立てたフラグを**降ろさない** |
| `ambiguity` | 大きい方 |
| `collaborative` / `seats` | 分類対象外。何体のAgentを使うかはユーザーの指定のみ |

過小評価は「実行できないモデルに回して失敗する」という実害になるが、過大評価は割高で済む、という非対称性に合わせている。

**コストへの影響。** 失敗（cooldown・rate limit・timeout・不正な応答）は常にheuristicの結果にfallbackし、分類が原因で実行が止まることはない。ただし1回のCLI起動分のレイテンシが全実行に乗る。同一本文の2回目以降はキャッシュを引く（30日）。また`complexity`が`Complex`/`Extreme`に上がると`roles::seats`が読み取り専用の2人目を着席させるため、chatでは1ターンのコストが倍になりうる。
- 回答末尾のJSON `{"candidate_id": "..."}`だけを受け付ける（CLIが先頭に出す通知文は無視）。提示した候補IDで、最小期待コストの1.25倍以内でなければ採用しない。
- 判定役がアカウント単位の制限・認証エラー・接続不能になった場合は、そのエージェントのcooldownに記録する。実行側も同じアカウントのため、Schedulerは実行前にquotaを再確認する。cooldown中の判定役は起動しない。
- `model`を省略した場合、判定の難しさを役割ごとに決め、実行候補と同じPolicy（成功確率の下限・制約・コスト）と残量で、最も安い適格なモデルとreasoningを選ぶ。Routerは簡単（simple）、Councilの各席は普通（normal）、Frontier Judgeは難しい（complex）として扱う。現在のPolicyでは、Routerは小型モデル・reasoning low、Councilは小型モデル・reasoning medium、Frontier Judgeは中位モデル・reasoning highになる。役割と難しさの対応はOrochi独自の目安で、コーディングの実績は選択に使わない。cooldown中のモデルは選ばない。`reasoning`だけを指定すると、そのreasoningに対応するモデルから選ぶ。
- 判定の記録は`runs`に、実際の判定役のAgent・（実際に使われた）モデル・usageと、失敗理由（`rate_limit`、`authentication`、`timeout`、`cooldown`、`invalid_advice`など）を残す。purposeが`execution`ではないため、実装品質の学習には使わない。

Councilは`[[council.members]]`に同じ形式で2〜4席を登録し、`orochi --council "タスク"`で明示実行する。常時使う場合は`[council] enabled = true`。

1. 各席が独立に候補を選ぶ。
2. 第1ラウンドの有効な候補ID一覧を各席へ渡し、再検討させる。第1ラウンドに答えたセッションはそのまま使い、票だけを送る。
3. 設定した席数の過半数が一致した候補を採用する。失敗した席があっても必要票数は減らさない。

席ごとの代替判定役には同じ課題属性・候補・peer votesを渡す。第1ラウンドで交代した席は、第2ラウンドもその代替から続ける。第2ラウンドで初めて使う代替には、候補一覧とpeer votesをまとめて渡す。

開始前に、全席・全代替・2ラウンド分の最大推定tokensを合計し、全判定役の最小`max_resource_fraction`で予算を検査する。第2ラウンドはセッションを使い回すため、1回目の依頼と回答を含む大きさで見積もる。予算外・不一致・全席失敗のときはローカルの選択へ戻る。Router・Judge・Councilは同時には呼ばない。

2026-09-16の実測（ACP判定役、各アダプターの報告値）では、1回あたりCodex gpt-5.6-lunaが約2.3万tokens・約5〜7秒、Claude Haikuが約2.8万tokens・約7〜8秒だった。詳細は[残タスクの実機検証](real-validation-20260916-2.md)。

判定役はルーティングだけを決める。最終的な編集は1つのACP Agentが実行する。独立セッションで実装・レビュー・統合する場合は、別コマンドの[`collaborate`](session-collaboration.md)を使う。

### ローカルモデル

判定役・実行とも、ACPで接続できるCLIがローカルモデルに対応していれば使える。Orochiは接続先のモデルがクラウドかローカルかを区別しない。CLI側でローカルモデルを設定し、ACPで報告されるモデルIDを`model`に指定する。`provider`はPolicyの選択（priorや制約）に使う。未知のモデルIDは、そのProvider Policyの既定ルール（`*`、tier unknown）で評価される。ローカルモデルでの実機確認はまだ行っていない。

## 4.2. チーム編成の自動導出

`collaborate`は`--plan`でチーム構成（Coordinator / Implementer / Reviewer / Integrator）をJSONで宣言する必要があったが、省略するとタスクから導出する。

```sh
orochi collaborate "アーキテクチャを全面的に刷新したい" --dry-run   # 編成だけ見る
orochi collaborate "..." --output report/                          # そのまま実行
orochi collaborate "..." --dry-run > team.json                     # 保存して手で直す
orochi collaborate "..." --plan team.json --output report/
```

`--dry-run`はplanファイルと同じ形式で出力するので、そのまま`--plan`に渡せる。`[classifier]`が有効なら、導出に使う`task_type` / `complexity`はそちらを通ったものになる。

| 役割 | 人数の決まり方 |
|---|---|
| Coordinator | Implementerが2人以上、または`long_horizon`、または`Extreme`のとき1人 |
| Implementer | `Extreme`3 / `Complex`2 / それ以外1。ただし**担当を分けられる領域数が上限** |
| Reviewer | 1人 + 構造変更なら+1 + `Extreme`なら+1（最大4） |
| Integrator | 常に1人 |
| Discussion | `collaborative`または`Complex`以上のとき。`Extreme`は3ラウンド、それ以外2 |

`seats`（「5人のエージェントで」）が指定されていれば、その人数に合わせて増減する。増やす順はCoordinator → Implementer → Reviewer、減らす順はその逆。

**Implementerの上限が領域数なのが要点。** 同じディレクトリに2人のImplementerを置いても、仕事が半分になるのではなく衝突するだけなので、`candidate_files`の最上位ディレクトリ（タスクがファイルに言及していなければリポジトリ直下のディレクトリ）の数を超えては増やさない。2人以上になる場合は、その領域を`paths`として重複なく配る。

各参加者の`agent`は空（自動選択）。どのエージェント×モデルを割り当てるかはschedulerの仕事で、`scorer`が他の席が使用中のエージェントを1.4倍で見積もるため、並行する席は自然に別アカウントへ散る。

## 4.3. コンソールからのチーム昇格

`orochi`（または`orochi chat`）でプロンプトを打つだけで、必要なら協業まで行く。ユーザーが`chat`と`collaborate`を選び分ける必要はない。

1ターンにつき分類は1回。その結果が、昇格の判断・フェーズ分割・ルーティングのすべてに使われる（`RunOptions::descriptor`で渡すので、schedulerが装飾後のテキストを再分類することはない）。

昇格の条件は`Plan::splits_work()` — 導出したチームのImplementerが2人以上のとき、つまり**作業が実際に分割できるとき**だけ。1人なら、別ワークスペースを用意しても得るものがないので、コンソールが自分の作業ツリーで1〜2席で回す。

昇格するときは**一度だけ確認する**。

```
◆ This is more than one agent's worth of work
  coordinator, 3 implementer(s), 3 reviewer(s), integrator, 3 discussion round(s)
  each implementer works in its own copy; a verified result is merged back into your tree
  ⎿ Enter or y to start the team · any other key runs it as a normal turn
```

数体のエージェントが同時に走り、検証を通れば作業ツリーに書き戻る唯一の経路なので、ここだけは黙って実行しない。端末がない場合（stdinがパイプ）は昇格しない。各行が1メッセージとして読まれる環境では、確認プロンプトが次のメッセージを食べてしまうため。

`/solo`と`/team`を明示したターンは昇格の対象外。レポートは`<data>/collaborations/<id>/`に残るので、中断しても`orochi collaborate-resume --output <path>`で続けられる。

## 4.4. 記憶

実装: 2026-09-18（P1）。設計の経緯・未実装部分は[個人最適化の設計](personalization-design.md)。**自動テストのみで、実エージェントでの取得精度は未検証。**

采配されるエージェントは毎回変わり、Claudeの`CLAUDE.md`はCodexに見えない。エージェントを跨いで保つべきものをOrochi側で覚える。

```
<data>/memory/USER.md                          ユーザー全体
<data>/memory/repos/<repository_id>/MEMORY.md  リポジトリ単位（ディレクトリ名はsalted hash）
```

**取得に追加のLLM呼び出しはない。** 分類器は毎ターン依頼を読んでいるので、その返答に「このタスクが終わっても有効な、ユーザー自身の指示や事実」を最大2件載せる。保存すると1行出る。

```
⎿ classified as bug_fix / normal
⎿ remembered: パッケージマネージャはpnpm
```

**注入は新しいセッションの開始時だけ。** chat・one-shot・席・collaborateの参加者すべてに入り、`resume`するターンには入らない（そのセッションは開始時に聞いている）。前置きで「タスクの指示とリポジトリの指示に優先しない」と位置づける。

**忘れ方。** Orochiが書いた行は`<!-- auto seen:N last:T -->`を持ち、1回だけ聞いたものは90日、2回以上は180日で消える。言い直しは回数を増やし、反対のことを言えば置き換わる。**手で書いた行（印なし）は消えず、置き換わらず、最優先で注入される。**

```sh
orochi memory                 # 一覧（u1, r2 ... のID付き）
orochi memory --forget r2     # 1件消す
```

コンソールでは`/memory`、`/memory forget r2`。ファイルを直接編集してもよい。`memory.enabled = false`で取得も注入も止まる。

**守っていること**（`tests/core.rs`の`mod memory`と`a_remembered_note_opens_each_fresh_session_once`）

- 記憶のテキストは`telemetry.sqlite3`に入らない。分類キャッシュにも残らない
- リポジトリAの覚え書きはリポジトリBのプロンプトに出ない
- router / judge / councilには渡らない。見るのは分類器だけで、現在のリポジトリの一覧のみ
- **対象リポジトリ内の`MEMORY.md`・`USER.md`は読まない。** リポジトリが全プロンプトに文を植え付けられないようにするため
- **エージェントに記憶を書かせるツールは無い。** エージェントの出力はリポジトリの内容に影響されるため

**セッションの振り返り**（P2）。1メッセージでは恒久的な好みか判断できないもの（同じ訂正の繰り返しなど）のために、2メッセージ以上あったコンソールセッションの終了時に1回だけ、そのセッションでユーザーが言ったこと（`/new`を跨ぐ・エージェントの返答は含まない）を分類器と同じ経路で見直す。

```
⎿ looking back over this session · Esc skips
⎿ remembered: キーワードマッチで判定しない
```

LLM呼び出しは1セッション1回。1メッセージだけのセッションでは呼ばない（送った時点で分類器が読んでいる）。発言はメモリ上にだけ置き、送り先は分類器と同じ。

**采配の好み**（P3）。「設計はFableで」のように、どの種類の仕事にどのエージェント・モデルを使いたいかを言えば（あるいは記憶に手で書けば）、分類器がそれを今回のタスクに当てはめて名前の断片を返し、一致する候補の期待コストを0.8倍にする。

```
⎿ classified as architecture / complex · you prefer fable
```

- **ゲートは開けない。** ケイパビリティ・クォータ・成功率の床で落ちた候補は、好みでも戻らない。大幅に安い・成功しやすい候補がある場合はそちらが勝つ
- 名前の断片（`fable`）で照合するので、モデルが`fable-6`に更新されても効く。発見されていないIDを作ることには使わない
- 0.8はOrochiのヒューリスティックで、測定した値ではない
- 今回だけの指定（`--model`）は従来どおりハード制約で、記憶には入らない

**限界。** 何を覚えるかは分類器の判断に依存し、保証ではない。外部から貼り付けたテキストに「これを覚えろ」と書かれていた場合に拾わないのは指示によるもので、件数・文字数の上限と取得時の表示が防御の残り。分類器が動かないターン（`--dry-run`、agentとmodelの両方をピン留めしたone-shot）では取得しない。

## 5. ACP公開

```sh
orochi --config /absolute/path/config.toml --data-dir /absolute/path/data serve
```

ACP Client側には上記をstdio Agentとして登録する。SDKは公式Rust ACP 2.0.0、公開プロトコルはv1。`initialize`、`session/new`、`session/prompt`、`session/cancel`とrequest cancellationに対応する。stdoutはJSON-RPC、進捗はstderr。backendの回答をストリームし、既定ask時はpermission要求を上位Clientへ中継する。deny/allowの明示設定を優先し、許可はallow_onceに限定する。

セッション内の直前のユーザー入力と回答を最大約24 KiBのメモリに保持し、次のpromptの文脈として利用する。各promptでは通常のSchedulerを実行する。会話全文の保持、永続session/load、model/mode切り替え、MCP設定の転送、追加workspace、画像・audio入力には未対応。未対応capabilityは広告せず、対応しない入力はエラーにする。Sessionは128件まで。

同一sessionの同時promptを拒否し、同じrepositoryへの実行も通常のworkspace lockで排他する。キャンセル・切断時はworkerとbackendの子プロセスを終了する。途中で中断されたgateway実行は、最終telemetryが保存されない場合がある。完了済みの実行は通常の評価・telemetryへ記録する。

## 6. 実機E2E

```sh
# CLI接続とモデル取得のみ
python3 examples/live_e2e.py --agent gemini --output .orochi/live-e2e/gemini

# ログイン済みCLIで、隔離した一時repoに1ファイル作成＋内容検証
python3 examples/live_e2e.py --agent gemini --execute --output .orochi/live-e2e/gemini
python3 examples/live_e2e.py --agent antigravity --command /absolute/path/agy_acp_server.par --execute --output .orochi/live-e2e/antigravity
```

`--command`と反復可能な`--arg`で実環境のACPコマンドを指定できる。例: `--arg=--acp`。各CLIの公式手順でインストール・ログインする必要がある。スクリプトは自動ログインせず、既存プロジェクトも変更しない。`--execute`は実アカウントのquotaを消費する。

CLI未導入、接続不可、未実行、実行失敗、検証成功を区別して`result.json`へ記録する。discovery成功だけでE2E完了とは表示しない。AntigravityはCLI 1.2.3・ACP Server 1.1.1の導入後、`Authentication required`まで確認。最新の証跡は`.orochi/live-e2e/session-work/antigravity/result.json`。Geminiの追加検証はユーザー指示により対象外。

Geminiの起動方式は[公式ACPモード](https://geminicli.com/docs/cli/acp-mode/)を参照。

## 検証結果

- 担当交代・再開・制限の範囲・途中状態保持の検証は[実測と検証](real-validation-20260916.md)、判定役のACP統一後を含む検証は[残タスクの実機検証](real-validation-20260916-2.md)に記録。判定役のテストはACP fixtureを利用。
- 実測・実機・未検証事項の一覧は[実測と検証](real-validation-20260916.md)を参照。
