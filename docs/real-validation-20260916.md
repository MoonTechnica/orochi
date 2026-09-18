# 実測・セッション協議の検証（2026-09-16）

## 1. 選択精度の実測

`examples/collect_benchmark.py`を追加。同じ開始ファイルから各候補を実行し、毎回独立したACPセッション・作業ディレクトリ・telemetry DBを用意します。モデル・reasoning・modeの一致、実評価、実usageが確認できた完全な組だけをデータセットへ出力します。候補の実行順を交互にし、失敗・欠落・認証エラーを記録します。

```sh
python3 examples/collect_benchmark.py \
  --suite examples/paired-suite.json \
  --adapter-cache /path/to/existing/orochi/adapters \
  --output /path/to/new/measurement --execute
orochi benchmark --input /path/to/new/measurement/dataset.json
orochi benchmark-tune --input /path/to/new/measurement/dataset.json --training-cases 4
```

係数探索は時系列の前半だけでalpha・prior_weight・explorationを選択し、選択履歴を引き継いだ後半で検証します。成功数を優先し、同じ成功数なら失敗ペナルティ込みコストを比較します。設定は自動変更しません。validationのラベルを変えても係数選択が変わらないことをテストしています。

### 実データ

Pythonの区間マージ課題を8回、Claude HaikuとCodex gpt-5.6-luna / mediumでそれぞれ実行しました。初回の課題文には「接する区間」の曖昧さがあったため採用せず、「[1,4]と[5,9]は別区間」と明記した再測定を採用しました。

| 実行候補 | 成功 | 総tokens | 1回あたり平均tokens |
|---|---:|---:|---:|
| Claude Haiku | 8/8 | 869,494 | 108,686.75 |
| Codex gpt-5.6-luna / medium | 8/8 | 207,058 | 25,882.25 |

以下は同じ実測データを使い、各方式が選んだ候補の結果だけで学習させた時系列replayです。

| replay方式 | 成功 | 総tokens | 成功1件あたりtokens |
|---|---:|---:|---:|
| Static | 8/8 | 207,058 | 25,882.25 |
| EWMA | 8/8 | 264,746 | 33,093.25 |
| Bandit（seed=42） | 8/8 | 264,746 | 33,093.25 |

前半4回で探索した係数は既定値から変わらず、後半4回では3方式とも4/4成功・105,234 tokensでした。**今回のデータではEWMA/Banditの優位性は確認できません。既定の係数は変更していません。**

8種類の独立課題ではなく、同じ課題族の反復です。小規模な動作確認であり、Provider間の一般的な性能比較や全タスクへの校正完了を意味しません。tokensは各ACPアダプターの報告値であり、料金ではありません。異なるProviderのcache/usage報告の差が含まれます。

証拠: `.orochi/live-e2e/session-work/paired-unambiguous/{dataset,evidence}.json`、`analysis/{replay,tuning}.json`。初回診断データは`paired-live/`に保存。

## 2. 実測で見つかった選択の不具合

- ACPの`data.details`に埋め込まれた429が単なる`Other: Internal error`になっていた。内部詳細を表示せずRateLimitとして分類。
- モデル明示時も全モデルの設定を試していたため、無関係なモデルの利用制限で接続全体が失敗していた。明示モデルだけを探索。
- `entirely`を`entire`として拾い、小関数をComplex判定していた。英単語の境界を区別。

## 3. 残量

Claude Code 2.1.272の公式`/usage`とstatuslineを実機確認。statuslineの5時間・週間の割合とリセット時刻を取り込みました。

`claude_usage`と`antigravity_usage`のネイティブCLIプローブを追加。Python 3 / POSIX PTYを使い、公式CLIの`/usage`表示だけを読み取ります。認証tokenの抽出やProviderの非公開API呼び出しは行いません。画面本文・アカウント名は保存しません。

Claudeはプローブ単体と`orochi quota --refresh`の両方で実残量を取得・保存できました。採取した実statusline JSONの`quota-ingest`取り込みも別の検証DBで確認しています。Antigravityは未認証のため、残量表示の実機パースは未検証です。未知のUIや取得不能はunknown/unavailableとして扱い、空の取得で既存の有効な観測を上書きしません。

```toml
[quota]
timeout_secs = 30
[[quota.probes]]
agent = "antigravity"
kind = "antigravity_usage"
command = "/absolute/path/to/agy"
```

Codex・Claudeは標準プリセットから自動検出。Antigravityは`agy`がPATHにあれば自動検出。未ログインのAntigravityで`quota --refresh`がOAuthを開始しないよう、先に`agy models`で認証を確認します。モデル名を推測せず、明示的なモデルIDとremaining/used方向が読み取れる場合だけ使用します。

## 4. セッション協議

[`session-collaboration.md`](session-collaboration.md)を参照。同じAgent・同じモデルの3つの独立セッション、レビュー受け渡し、レビュー中の編集の隔離、統合後の評価をfixtureで確認。

**実Claude Haikuでも実装→レビュー→統合が成功しました。** ASCII slug関数を作成し、最終ローカル評価を通過しました。実装者・レビュー者・統合者のACPセッションIDはすべて別です。レビュー役はファイル評価の対象ではないため、単独のoutcomeは`partial_success`、最終成果物は`success`です。

証拠は`.orochi/live-e2e/session-work/council-live/report-final.json`、成果物は`council-live/result-final/integrator/slug.py`。

## 5. Antigravity / Gemini

- Antigravity CLI 1.2.3と公式ACP Server 1.1.1を`.orochi/tools/antigravity/`へ導入。
- ACP接続は`Authentication required`まで到達。タスク実行・モデル選択・評価はGoogleログイン後の検証が必要。
- Gemini CLIはユーザー指示により以降の対応対象から除外。導入確認時点の0.59.0はAPIキー未設定で止まっており、ログイン・実行は行っていない。
- Antigravityの公式インストーラは専用インストール先のPATHを`.zshrc`・`.zprofile`・`.bash_profile`へ追加した。

## 6. ACP gateway

`examples/gateway_live_e2e.py`で公開`orochi serve`を実Codexに接続。ACP initialize/new/prompt、102件のstream update、生成ファイル、ローカル評価の成功を確認。証拠は`.orochi/live-e2e/session-work/gateway-live.json`。

Zed UIでのE2Eは、全体設定への一時的な外部エージェント追加が自動承認レビューに拒否されたため、明示承認待ち。設定例は`zed-gateway/zed-agent-entry.json`に作成済み。このACPクライアントによる実サービス試験をIDE UI試験と同一扱いしない。

## 7. 担当交代追加前のチェック

- `cargo test --locked`: Rust統合テスト66件、および内部から実行するPython残量パーサー4件が成功。
- `cargo clippy --locked --all-targets -- -D warnings`、`cargo fmt --check`が成功。
- キャンセル試験は起動確認の2秒上限を超えて失敗したため、状態を確認しながら最大10秒待つ形へ変更。起動失敗時は子プロセスを回収し、診断ログを表示する。
- セッションの作業コピーから`.env*`を除外し、大小文字違いの参加者ID衝突と予約ディレクトリ名の使用を拒否。隔離・除外の回帰テストを追加。

残る外部条件はAntigravityのGoogleログイン、Zedの一時設定追加の承認、HTTP Judge/Councilの実サービス接続設定。多様な課題による追加校正、任意の宛先への多往復協議は今回の小規模実測・固定フローの範囲を超える残タスク。

## 8. 司令塔を含む担当交代の追加実装

`collaborate`に任意の`coordinator`役を追加。最初と各工程の終了後に計画・判断・未解決事項を更新する。司令塔・実装・レビュー・統合の全役割で、指定した候補が利用不能になった場合に利用可能なAgent／モデルへ交代する。候補範囲・固定指定・Policy・推定成功率・残量・試行上限を守る。

`report.json`をschema version 2の再開可能な状態としてアトミック保存する。途中回答は受信ごとに保存し、失敗した担当の編集済みファイルと回答・検証結果を次の独立セッションへ渡す。全候補失敗・キャンセル後は`collaborate-resume`で未完了工程から再開する。完了済み工程は再実行せず、明示キャンセル時には自動で代替AIを起動しない。

HTTP Router／Frontier Judge／Councilにも最大3個の代替endpointを設定できるようにした。402／429等のエラーや無効回答時に同じ判断材料を渡し、Councilでは1担当1票と過半数を維持する。代替候補を含む総予算を事前に制限する。これらのHTTP交代はfixture検証であり、実HTTPサービスの認証・接続確認とは区別する。

### 実Codexによる継続確認

司令塔のクレジット不足をfixtureで発生させ、ログイン済み実Codexへ自動で交代した。初回はCodexのCLI警告が回答の先頭に混ざり、厳密なJSON読み取りが失敗した。末尾の完全な管理状態JSONを検証して読み取る処理に修正し、**保存済みレポートから再開して7工程すべてを完了**した。

実Codexが司令塔4回・実装1回・レビュー1回・統合1回を担当し、統合後の`result.txt`が`OROCHI_FAILOVER_OK`＋改行1文字と完全一致することをローカル評価と独立確認の両方で検証した。元のテスト作業ツリーに変更がないことも確認済み。

これは**模擬的な制限エラー→実Codexの継続実行**の検証であり、実Claudeアカウントのクレジットを枯渇させた試験ではない。

証拠: `.orochi/live-e2e/session-failover/verification.json`、`result/report.json`、`before-resume.json`。使用方法と保存範囲は[セッション協議](session-collaboration.md)を参照。

追加実装後はRustテスト81件、内部から呼び出すPython残量パーサー4件がすべて成功。司令塔のCtrl-C中断、途中回答の保存、同じ工程からの再開、モデル限定の制限を検出して他モデルを残すケースも含む。Clippy（`--all-targets -- -D warnings`）と`cargo fmt --check`も成功。

## 公式資料

- [Claude statusline](https://code.claude.com/docs/en/statusline)
- [Antigravity /usage](https://www.antigravity.google/docs/cli/commands/usage)
- [Antigravity CLI導入](https://www.antigravity.google/docs/cli/install)
- [ACP Registry](https://github.com/agentclientprotocol/registry)
- [Zedの外部Agent設定](https://zed.dev/docs/ai/external-agents)
