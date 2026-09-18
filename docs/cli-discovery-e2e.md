# CLI本体の自動検出・実機E2E（2026-09-15）

## 問題と変更

従来は`claude-agent-acp`の存在だけでClaudeのインストール状態を判定していたため、`claude`本体が存在しても候補から除外されていた。

- CLI本体・ACPアダプター・接続準備の状態を分離した。
- Claude/CodexのCLI本体があれば、不足するACPアダプターをOrochiの`adapters/`キャッシュへ自動導入する。
- 既知の4プリセットを省略した設定にも補完する。同じIDの明示設定と`enabled = false`を優先する。
- `agents`はダウンロードせず、`adapter_required`等の状態を表示する。実行・discovery時に準備し、失敗理由を表示する。
- 固定バージョン、専用npmキャッシュ、install scripts無効、排他制御、原子的な導入、別枠の準備タイムアウトを使用する。

自動検出の対象はCodex、Claude Code、Gemini CLI、Antigravity ACP。その他はカスタムACPコマンドを登録する。任意のCLIへ未知のプロトコルを推測して送る実装ではない。

## 実機確認

macOS上でPATHから利用できたCoding CLIはCodexとClaude Code。Gemini/Antigravityは未導入。

| 確認 | 結果 |
|---|---|
| Claude本体だけでのinventory | `installed: true`, `adapter_required` |
| 不足アダプターの自動導入 | `@agentclientprotocol/claude-agent-acp@0.77.0`を専用キャッシュへ導入成功 |
| 既存CLIの利用 | `CLAUDE_CODE_EXECUTABLE`に検出した`claude`を指定 |
| ClaudeのACP discovery | `default`, `opus[1m]`, `claude-fable-5-1[1m]`, `sonnet`, `haiku` |
| CodexのACP discovery | `gpt-5.6-sol`, `gpt-6-astra`, `gpt-5.6-terra`, `gpt-5.6-luna`, `gpt-5.5` |
| 元のWebアプリ依頼文でのdry-run | Claude/Codexの計10候補がルーティング対象 |
| Claudeへの実行委譲 | `--agent claude`でHaikuを選び、1ファイル作成・内容検証に成功 |
| Claude実行の所要時間 | discovery込み10.46秒、1試行、終了コード0 |

元の依頼文では`gpt-5.6-luna / medium`が先頭で、`haiku`も同じ推定コストで候補に含まれた。今回は候補登録と実行接続の検証であり、Provider間の品質・費用の優劣を示すベンチマークではない。Claudeへの委譲確認には明示的な`--agent claude`を使用した。

## 再現用のローカル成果物

`.orochi/live-e2e/native-discovery/`配下に次を保存した（Git管理対象外）。

- `evidence/discovery.json`: 実Agentのモデル・reasoning・mode一覧
- `evidence/route.json`: 元のWebアプリ依頼文に対する全候補とスコア
- `evidence/claude-run.log`, `claude-result.json`, `runs.json`: Claude実行結果
- `config.toml`, `task.txt`, `repo/result.txt`: 実行条件と生成ファイル

検証用アプリへの追加修正は行っていない。

## 回帰テスト

ネットワーク不要のテストで、CLI本体のみの検出、npm不足と導入無効の区別、明示設定の維持、同時導入の排他、キャッシュ再利用、導入失敗後の再試行、CLI実行から候補登録・ファイル作成までを検証する。

全36テスト、Clippy（警告をエラー扱い）、Rustfmtのチェックが成功。リリースビルドとアカウント不要のデモも成功した。

## 接続方式の出典

- [Codex ACPのCODEX_PATH](https://github.com/agentclientprotocol/codex-acp/blob/main/README.md)
- [Claude Agent ACPのCLAUDE_CODE_EXECUTABLE](https://github.com/agentclientprotocol/claude-agent-acp/blob/main/src/acp-agent.ts)
