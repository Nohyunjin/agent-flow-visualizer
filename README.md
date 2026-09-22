# Agent Flow

Claude Code와 Codex의 로컬 세션을 한 화면에서 탐색하는 **Rust TUI**입니다. 이미 실행 중인 에이전트의 JSONL 로그를 읽으므로 에이전트를 다시 실행하거나 훅을 설치할 필요가 없습니다.

```sh
cargo run --release
```

설치해서 어디서든 실행하려면:

```sh
cargo install --path . --locked
agent-flow
```

설치 후 `command not found: agent-flow`가 나오면 Cargo 설치 폴더가 PATH에 없는 것입니다. 즉시 실행하려면 `~/.cargo/bin/agent-flow`를 사용하세요. 현재 터미널에서 이름으로 실행하려면 `export PATH="$HOME/.cargo/bin:$PATH"`를 먼저 실행합니다. 이 PATH 설정을 셸 설정 파일에 추가하면 새 터미널에도 적용됩니다.

Rust 1.88 이상, UTF-8 터미널이 필요합니다. macOS에서 실제 로그와 PTY 동작을 검증했습니다. 145열 이상이면 세 패널을 나란히 표시하고, 95~144열에서는 상세 패널을 아래에 표시합니다. 더 작은 창에서는 `1` / `2` / `3`으로 패널을 전환합니다.

## 무엇을 볼 수 있나요?

```text
 AGENTS                       FLOW                             INSPECT
 ▸ 결제 오류 수정 [Codex]      ◆ 사용자 요청                    exec_command
 └─ reviewer [READY]          ├ exec_command → 결과             INPUT
 ▸ 접근성 점검 [Claude]       ╞ spawn_agent → reviewer            { "cmd": "cargo test" }
 └─ tests [WORKING]            │  ├ exec_command → ERROR         RESULT
                              │  ↔ send_message → 부모          실패한 테스트와 출력
                              ├ apply_patch → 결과
                              └ exec_command → WAIT
```

- **에이전트 트리**: Claude/Codex, 부모·자식 관계, 작업 설명, 상태, 마지막 활동 시점. `Main ~20.0k/200.0k 90% free`처럼 각 에이전트의 현재 컨텍스트 사용량 / 한도와 남은 비율을 표시합니다. 다음 줄의 `351 ev`는 보관 중인 이벤트 수입니다.
- **흐름**: 요청, 응답, 도구 호출, 반환값, 에이전트 생성·메시지를 시간순으로 연결합니다. 부모를 선택하면 자식의 작업도 함께 표시합니다.
- **상세**: 선택한 이벤트의 입력, 반환값, 기록된 소요 시간, 호출 ID, 세션 ID와 원본 로그 경로. 서브 에이전트 링크에서는 `Enter`로 그 에이전트에 들어갑니다.
- **실시간 추적**: 기본 1초 간격으로 새 파일과 추가된 줄을 읽습니다. 로그 읽기는 별도 스레드에서 수행합니다. 같은 세션을 탐색 중에는 선택한 이벤트를 유지하고, `f`를 켜면 최신 이벤트를 따라갑니다. Agents에서 다른 세션으로 이동하거나 이전 세션으로 돌아오면 현재 필터에 맞는 최신 FLOW 이벤트를 선택합니다. 이때 Follow 설정은 유지됩니다.
- **검색·필터**: 세션/작업 폴더 검색, 이벤트 입력·결과 검색, 도구만 보기, 오류만 보기, 제공자별 보기.

안전한 샘플 데이터로 먼저 탐색하려면:

```sh
agent-flow --demo
# 설치 전에는 cargo run -- --demo
```

## 키보드 조작

| 키 | 동작 |
|---|---|
| `Tab` / `Shift-Tab`, `1` `2` `3` | 에이전트 / 흐름 / 상세 패널 이동 |
| `j` `k`, `↑` `↓` | 선택 이동, 상세 내용 스크롤 |
| `g` `G`, `Home` `End` | 처음 / 마지막 |
| `PageUp` / `PageDown` | 10행 이동 |
| `Enter` | 흐름 열기, 상세 보기, 연결된 에이전트로 이동 |
| `b` / `Backspace` | 부모 에이전트로 이동 |
| `[` / `]` | 어느 패널에서든 이전 / 다음 이벤트 |
| `/` | 현재 패널 검색 (`Enter` 적용, `Esc` 지우기) |
| `p` | 전체 → Codex → Claude |
| `a` | 전체 / 최근 작업 중인 세션 |
| `s` | 선택한 에이전트만 / 하위 에이전트 포함 |
| `t` / `e` | 도구만 / 오류만 표시 |
| `f` | 최신 이벤트 따라가기 |
| `Space` | 화면 업데이트 일시정지 / 재개 |
| `r` | 즉시 다시 스캔 |
| `?` | 도움말과 로그 접근 경고; `j/k`로 스크롤 |
| `q` / `Ctrl-C` | 종료 |

## 로그 위치와 범위

| 제공자 | 기본 위치 | 해석하는 기록 |
|---|---|---|
| Codex | `$CODEX_HOME/sessions`, 기본 `~/.codex/sessions` | `session_meta`, `turn_context`, `response_item`, `event_msg` |
| Claude Code | `$CLAUDE_CONFIG_DIR/projects`, 기본 `~/.claude/projects` | 세션 JSONL 및 `<session>/subagents/agent-*.jsonl` |

```sh
agent-flow --codex-dir /path/to/codex/sessions \
           --claude-dir /path/to/claude/projects \
           --max-sessions 200 --max-events 3000 --interval 1000
```

기본으로 수정 시각이 최근인 **120개 로그와 확인된 부모 로그**를 읽고, 에이전트당 최근 **1,500개 이벤트**를 보관합니다. 더 오래된 활동을 찾으려면 위 옵션을 늘리세요. 초기 대용량 파일 읽기 진행 상태는 하단 `loading`에 표시합니다.

Codex의 부모 이력 복사본은 `subagent_history_start_ordinal` 경계로 제외합니다. Claude의 자식 로그 경로와 `Agent`/`Task` 결과의 `agentId`를 연결하고, 확인된 생성 기록이 있으면 직접 부모를 교정합니다. 도구 입력과 결과는 `call_id` / `tool_use_id`로 연결합니다. Codex가 기록한 중첩 명령 실행 완료 이벤트도 표시합니다.

## 상태를 읽는 법

이 앱의 상태는 **로그에 남은 활동**을 뜻합니다. OS 프로세스가 실제로 살아 있는지 확인하는 지표는 아닙니다.

- `WORKING`: 열린 턴이며 120초 이내 활동이 기록됨.
- `QUIET`: 열린 턴이지만 120초 넘게 새 활동이 없음. 긴 도구 실행, 승인 대기, 중단 등을 구분할 근거가 부족합니다.
- `READY`: 명시적인 턴 완료가 기록됨. 세션 프로세스 종료라는 뜻은 아닙니다.
- `UNKNOWN`: 턴 상태를 판단할 기록이 없음.
- `WAIT`: 대응하는 도구 결과가 아직 없음. 오래된 로그의 누락·중단도 포함합니다.
- `RETURN`: 결과가 반환됨. 작업의 의미적 성공을 보장하지 않습니다.
- `ERROR`: 명시적 오류 플래그, 실패 상태, 0이 아닌 종료 코드가 기록됨.

## 현재 컨텍스트와 남은 여유

Agents의 `Main` / `Sub`는 해당 에이전트의 **가장 최근 요청에서 기록된 입력 + 출력 토큰**으로 현재 컨텍스트 크기를 추정합니다. 예를 들어 `Main ~20.0k/200.0k 90% free`는 약 20,000토큰을 사용했고 200,000토큰 한도에서 약 90%가 남았다는 뜻입니다. 여러 요청의 입력을 계속 더한 누적 소비량은 화면에 표시하지 않습니다. `1k = 1,000`, `1M = 1,000,000`입니다.

- 메인과 서브 에이전트의 컨텍스트는 각각 표시합니다. 하위 흐름을 함께 보더라도 부모에 자식의 사용량을 더하지 않습니다.
- `~`는 마지막 요청을 기준으로 한 추정치입니다. 그 이후 추가된 사용자 메시지나 도구 결과는 다음 사용량 기록이 남기 전까지 반영되지 않습니다. 구독 한도나 요금 사용률을 뜻하지 않으며, 자동 압축은 표시된 컨텍스트 한도에 도달하기 전에 발생할 수 있습니다.
- Codex는 `last_token_usage` 또는 요청별 `usage`에서 사용량을, `model_context_window`에서 기록된 한도를 읽습니다. 복사된 부모 이력은 제외합니다.
- Claude 입력에는 일반 입력과 캐시 생성·읽기를 합산합니다. 캐시도 현재 요청의 컨텍스트를 차지합니다. 이 구분은 [Claude의 사용량 필드 정의](https://platform.claude.com/docs/en/build-with-claude/prompt-caching#tracking-cache-performance)를 따릅니다.
- Claude 한도는 로그의 명시적인 크기, 기록된 모델의 `[1m]` 태그, 지원하는 모델의 문서상 기본값 순으로 판단합니다. 기본값을 사용한 경우 Inspect에 `model default; settings may override`라고 표시합니다. 실제 한도는 설정이나 제공자에 따라 다를 수 있습니다. [모델 및 컨텍스트 설정](https://code.claude.com/docs/en/model-config), [Claude 상태 표시줄의 컨텍스트 필드](https://code.claude.com/docs/en/statusline)를 참고하세요.
- 한도를 판단할 수 없으면 `~12.9k / ? ctx`로 표시하고 남은 비율을 계산하지 않습니다. 사용량이 없으면 `—`를 표시합니다. 압축 기록을 만나면 이전 사용량을 지우고 다음 요청의 사용량을 기다립니다.
- Inspect의 **`CONTEXT AT EVENT`**는 해당 이벤트 시점까지 기록된 값으로 고정됩니다. 새 요청이 실행되어도 과거 이벤트의 숫자는 바뀌지 않습니다. 도구 호출은 호출 시작 시점의 값이며, 상세에 사용량 기록 시각과 한도 근거를 함께 표시합니다. 당시 기록이 없으면 이후 값으로 채우지 않습니다.

JSON 스냅샷의 `token_usage`는 별도로 유지하는 누적 처리량이며 `context`와 의미가 다릅니다. 누적 처리량은 캐시 입력의 반복 처리도 포함하므로 현재 컨텍스트 한도보다 훨씬 클 수 있습니다.

## 데이터와 한계

로컬 파일을 읽기 전용으로 엽니다. 네트워크 요청, 로그 업로드, 에이전트 실행·중지, 설정 수정 기능은 없습니다. `Space`는 화면만 멈추고 원래 에이전트는 계속 실행합니다.

내부 로그 형식은 버전에 따라 달라질 수 있습니다. 지원하지 않는 레코드는 건너뛰고, 잘못된 JSON과 8 MiB를 넘는 레코드는 개수로 표시합니다. 아직 줄바꿈이 기록되지 않은 마지막 줄은 완성될 때까지 기다립니다. 파일 교체와 길이 감소 시 다시 읽습니다. 암호화되거나 숨겨진 추론은 표시하지 않습니다. 로그에 기록되지 않은 내부 호출이나 서브 에이전트 결과를 복원할 수는 없습니다. `codex exec --json` stdout 이벤트 스트림, 원격 세션, Claude 웹 채팅은 별도 입력 형식이므로 이 버전의 자동 수집 대상이 아닙니다.

한 이벤트의 입력/결과 표시는 각각 64 KiB로 제한하며 잘린 내용에 표시를 붙입니다. 전체 내용은 상세 패널에 표시된 원본 로그에 있습니다. 출력의 ANSI 제어 문자는 제거하지만, 로그 자체에 담긴 비밀값은 마스킹하지 않습니다. 화면 공유나 JSON 내보내기에는 실제 대화·도구 출력이 포함될 수 있습니다.

비대화형 확인:

```sh
agent-flow --demo --render 160x42  # 텍스트 화면
agent-flow --snapshot             # 현재 수집 범위의 JSON; 대화 내용 포함
```

로그 저장 구조 참고: [Claude Code 디렉터리 문서](https://code.claude.com/docs/en/claude-directory), [Codex App Server의 스레드와 로컬 기록](https://learn.chatgpt.com/docs/app-server). 파서의 구체적인 필드 지원은 실제 로컬 로그와 합성 테스트를 기준으로 검증했습니다.

## 개발과 검증

```sh
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
python3 scripts/smoke_tui.py target/release/agent-flow
```

`parser.rs`가 제공자별 로그를 `model.rs`의 공통 이벤트로 변환합니다. `source.rs`는 파일 발견과 증분 읽기, `app.rs`는 선택·탐색·필터, `ui.rs`는 반응형 TUI를 담당합니다. 테스트는 호출 결과 연결, 부모 이력 제외, 파일 추적, 보존 한도, 상태, 검색·탐색과 화면 크기 변경을 검증합니다. 데모 데이터에는 실제 사용자 로그가 포함되지 않습니다.
