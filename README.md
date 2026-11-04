# qmd-memory

Claude·Codex 세션에서 나온 프로젝트 방향, 결정과 이유, 문제 제기, 실패와 해결 경험을 `D:\doc\knowledge`의 주제별 기억 문서로 자동 기록하는 Windows 트레이 앱입니다. 후속 에이전트는 qmd로 이 문서를 찾아 씁니다. 프로젝트 루트는 `D:\src\personal\qmd-memory`이고, 분리 경위와 이전 결정은 [docs/origin-and-scope.md](docs/origin-and-scope.md), 이번 설계와 결정 기록은 [docs/superpowers/specs/2026-11-04-tray-app-design.md](docs/superpowers/specs/2026-11-04-tray-app-design.md)에 있습니다.

## 동작 흐름

```
세션 종료(SessionEnd, Claude는 PreCompact도)
  → 전역 훅: qmd-memory-cli.exe hook <claude|codex>   표준입력 JSON에서 원문 경로를 읽어 SQLite 큐에 등록하고 바로 종료
  → 트레이 앱(qmd-memory.exe)의 처리 스레드          5초마다 큐 확인. 훅이 큐에 넣은 작업만 처리
      1. 원문 읽기(.jsonl, 압축된 .jsonl.zst도 읽음), 처리한 앞부분이 바뀌지 않았는지 해시 확인
      2. 작업 폴더(cwd) → git 저장소(worktree면 원래 저장소) → 프로젝트 이름
      3. 지난 처리 위치 이후 발언(최대 30,000자)과 앞 문맥, 해당 프로젝트·common 기억 문서 준비
      4. 기록 모델 호출(codex exec 또는 claude --print) → JSON 변경안
      5. 경로·인용문·출처 검사 → 대화 사본 저장 → 기억 문서 저장 → qmd update/embed → 처리 위치 진행
  → 시작·완료·실패 시 Windows 알림, 트레이 팝오버에 상태 표시
```

앱이 꺼져 있어도 훅은 큐에 등록합니다. 앱을 켜면 쌓인 작업을 처리합니다. 앱은 세션 폴더를 직접 훑지 않으므로, 강제 종료 등으로 종료 훅이 실행되지 않은 세션은 기록하지 않습니다. 필요하면 `qmd-memory-cli scan`으로 직접 큐에 넣을 수 있습니다.

## 구성

| 경로 | 내용 |
|---|---|
| `crates/core` | 원문 해석, 처리 구간, 변경안 검사, 문서·대화 사본 작성, SQLite 큐, qmd·모델 실행, 링크 이전 |
| `crates/cli` | `qmd-memory-cli.exe`. 훅 접수와 수동 처리 명령 |
| `src-tauri` | `qmd-memory.exe`. 트레이 아이콘, 팝오버 창, 처리 스레드, 알림 |
| `src` | 팝오버 화면(React 18 + TypeScript + Vite + Zustand, lucide 아이콘, Pretendard). 색상 토큰과 토글·세그먼트 형태는 `tc-toolbox`의 디자인 토큰을 따르고, 라이트·다크 테마를 지원합니다. |
| `crates/core/src/recorder-prompt.md` | 기록 모델 지침. 실행 파일에 포함됩니다. |

구조와 규약은 `D:\src\tools`의 Tauri 앱(`tc-toolbox`의 트레이 상주 방식)을 따릅니다. ExampleCo Toolbox 도구로 등록하지 않는 별도 앱입니다.

## 빌드와 설치

필요한 것: Rust(MSVC), Node.js, pnpm, Visual Studio 빌드 도구, WebView2. 기록 모델 실행에는 로그인된 Codex 또는 Claude CLI가, 색인 갱신에는 `C:\bin\qmd`의 qmd가 필요합니다.

```powershell
pnpm install
pnpm tauri build            # 설치본: target/release/bundle/nsis/qmd-memory_0.1.0_x64-setup.exe
pnpm tauri build --no-bundle  # 설치본 없이 target/release/qmd-memory.exe와 qmd-memory-cli.exe만
```

`pnpm build`가 먼저 `qmd-memory-cli`를 릴리스로 빌드해 `src-tauri/binaries/`에 복사합니다. Tauri 설치본은 이 CLI를 앱과 같은 폴더에 설치합니다. Windows 알림이 앱 이름으로 표시되려면 설치본으로 설치해야 합니다. `target/` 아래 실행 파일로 실행하면 알림이 PowerShell 이름으로 표시됩니다.

## 훅 등록

팝오버 설정 화면의 "훅 연결"에서 Claude Code와 Codex의 연결 상태(연결됨, 다른 경로, 연결 안 됨)를 보고, "연결"과 "해제" 버튼으로 연결하거나 해제합니다. 같은 일을 `qmd-memory-cli.exe connect <claude|codex>`, `disconnect <claude|codex>`로도 할 수 있습니다. 앱은 사용자가 누르기 전에는 전역 설정 파일을 바꾸지 않습니다. 해제는 qmd-memory 훅만 지우고, 연결 때 새로 만든 이벤트 항목이 비면 그 항목도 지웁니다. 설치본으로 설치한 앱을 제거하면 제거 프로그램이 Claude·Codex의 qmd-memory 훅을 해제합니다(업데이트를 위한 재설치일 때는 해제하지 않습니다).

- Claude: `~/.claude/settings.json`의 `hooks`에 `SessionEnd`, `PreCompact` 항목을 추가합니다.
- Codex: `~/.codex/hooks.json`의 `hooks`에 `SessionEnd` 항목을 추가합니다. Codex에서 `/hooks`로 새 훅을 신뢰로 표시해야 실행됩니다.

기존 훅과 다른 설정 키는 순서까지 그대로 두고 배열 끝에 추가합니다. 다른 설치 위치를 가리키는 이전 qmd-memory 훅은 새 경로로 바꿉니다. 바꾸기 전 파일은 같은 폴더에 `<파일>.qmd-memory-backup-<시각>`으로 남깁니다. 기록 모델 프로세스에는 `QMD_MEMORY_RECORDER=1`을 넣으므로, 그 세션이 끝나도 훅은 등록하지 않습니다.

## 데이터 위치

| 경로 | 내용 |
|---|---|
| `%LOCALAPPDATA%\dev.jyjo.qmd-memory\settings.json` | 설정. 처음 실행한 시각(`since`)은 수동 `scan` 명령이 그 이전 세션을 건너뛰는 기준입니다. |
| `%LOCALAPPDATA%\dev.jyjo.qmd-memory\state.sqlite` | 작업 큐, 처리 위치, 실패 횟수, 최근 기록 |
| `%LOCALAPPDATA%\dev.jyjo.qmd-memory\reviews\` | 최근 50개 처리의 모델 입력·응답 |
| `%LOCALAPPDATA%\dev.jyjo.qmd-memory\migration\` | 2026-11-04 링크 이전 전 기억 문서 백업과 원본 세션 백업 |
| `D:\doc\knowledge\projects\<프로젝트>\`, `common\` | 기억 문서(qmd 색인 대상) |
| `D:\doc\knowledge\sources\<claude|codex>\<세션 ID>.md` | 기억에 쓰인 세션의 대화 사본(qmd 색인 대상 아님) |

## 처리 규칙

- **실패 처리:** 실패한 작업은 다음 시도 시각을 정해 뒤로 미루므로 다른 작업을 막지 않습니다. 실행 실패·시간 초과는 1분부터 두 배씩 최대 1시간 간격으로 8번까지, 모델 응답 검사 실패는 3번까지 다시 시도합니다. 원문이 없거나 처리한 앞부분이 바뀐 경우처럼 다시 해도 같은 결과인 실패는 바로 `실패`로 두고, 팝오버의 "다시 시도"로만 다시 넣습니다.
- **긴 발언:** 12,000자를 넘는 발언은 앞 3/4과 뒤 1/4만 모델에 보내고 가운데를 `[… 중략 …]`으로 표시합니다. 인용문은 원래 발언 전체와 대조합니다.
- **기록하지 않는 세션:** Codex 하위 에이전트 세션(세션 파일의 `subagent` 표시), Claude `subagents` 폴더의 세션, git 저장소 밖에서 진행한 세션은 `건너뜀`으로 둡니다.
- **에이전트가 보낸 입력:** Orca나 다른 에이전트가 보내는 고정 형식 메시지는 입력마다 `agent`로 표시합니다. 대상은 "You are working inside Orca", "You are a dispatched worker", "Please carry out this task from my Orca coordinator", "Continue work from the prior Orca session", "The prior provider session is read-only context", "Original agent: ", "Another Claude session sent a message"로 시작하거나 첫 줄이 "You have N orchestration message(s)"인 입력입니다. 새로 들어온 구간의 입력이 모두 `agent`이면(입력 없이 답변만 있는 구간은 직전 입력이 `agent`이면) 기록 모델을 부르지 않고 넘어갑니다. 그 밖의 입력이 하나라도 있으면 구간 전체를 기록 모델에 보내고, 어느 입력이 사람이 직접 친 말인지와 무엇을 기록할지는 기록 모델이 판단합니다. `agent` 입력은 사용자 결정의 근거로 쓸 수 없습니다.
- **새 프로젝트:** 기억 폴더가 없는 저장소의 첫 기록이면 `projects/<이름>/`을 만들고 qmd 컬렉션 `project-<이름>`을 추가합니다.
- **기존 문서 입력:** 해당 프로젝트와 common 문서가 60,000자 이하이면 모두 보냅니다. 넘으면 qmd 벡터 검색으로 관련 문서를 골라 한도까지 본문을 보내고, 나머지는 경로와 제목만 보냅니다. 모델이 본문 없는 문서를 고쳐야 하면 `needs_notes`로 요청하고, 앱이 그 본문을 넣어 한 번 더 호출합니다. 본문을 받지 않은 기존 문서를 고치는 변경안은 거부합니다.
- **반영:** 검토 단계 없이 자동 반영합니다. 변경안 작성 이후 누가 문서를 고쳤으면 반영하지 않고 실패로 둡니다.

## CLI

```powershell
qmd-memory-cli status            # 큐 상태와 실패한 작업
qmd-memory-cli retry <작업 ID>    # 실패·건너뜀 작업을 다시 대기열에
qmd-memory-cli enqueue claude <원문 경로>
qmd-memory-cli scan              # 훅이 놓친 세션을 직접 큐에 등록(앱은 자동으로 하지 않음)
qmd-memory-cli work [--once]     # 트레이 앱이 꺼져 있을 때 수동 처리
qmd-memory-cli hooks             # 훅 연결 상태 출력
qmd-memory-cli connect claude    # 전역 훅 설정에 연결(이전 파일 백업)
qmd-memory-cli disconnect claude # 전역 훅 설정에서 qmd-memory 훅 제거(이전 파일 백업)
qmd-memory-cli migrate-links [--apply] [--backup <폴더>]   # 원본 세션 링크를 대화 사본 링크로 이전
```

`work`는 트레이 앱이 실행 중이면 거부합니다(같은 잠금 파일을 씁니다).

## 테스트

```powershell
cargo test --workspace   # Rust 고정 입력 검사
pnpm test                # 팝오버 화면 검사(vitest)
pnpm typecheck
```

테스트는 고정 입력, 임시 폴더와 지정한 시각만 씁니다. 모델·qmd·네트워크·현재 시각에 의존하지 않습니다.

## 이전 Python 프로토타입

`memory.py`, `runtime.py`, `tests/test_memory.py`, `build.ps1`, `standalone/`, `work/`, `work-auto/`, `examples/`, 루트의 `recorder-prompt.md`는 작업 스케줄러로 실행하던 이전 프로토타입입니다. 새 앱은 이 파일을 쓰지 않습니다. 검증 기록은 [VALIDATION.md](VALIDATION.md)에 있습니다.
