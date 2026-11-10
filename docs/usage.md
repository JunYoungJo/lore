# Lore 사용 안내

[Lore 소개와 시작하기](../README.ko.md) · [여러 PC 설정](multiplatform.md) · [동기화 검토](sync-review.md)

앱은 `lore`, CLI는 `lore-cli`, 앱 식별자는 `io.github.junyoungjo.lore`입니다. 아래 명령과 경로는 이 이름을 기준으로 합니다.

## 훅 등록

팝오버 설정 화면의 "Hook connections"에서 Claude Code와 Codex의 연결 상태(연결됨, 다른 경로, 연결 안 됨)를 보고, "Connect"과 "Disconnect" 버튼으로 연결하거나 해제합니다. 같은 일을 `lore-cli.exe connect <claude|codex>`, `disconnect <claude|codex>`로도 할 수 있습니다. 앱은 사용자가 누르기 전에는 전역 설정 파일을 바꾸지 않습니다. 해제는 lore 훅만 지우고, 연결 때 새로 만든 이벤트 항목이 비면 그 항목도 지웁니다. Windows NSIS 설치본을 제거하면 제거 프로그램이 Claude·Codex의 lore 훅을 해제합니다(업데이트를 위한 재설치일 때는 해제하지 않습니다). macOS·Linux에서는 앱을 지우기 전에 훅을 해제합니다.

- Claude: `~/.claude/settings.json`의 `hooks`에 `SessionEnd`, `PreCompact` 항목을 추가합니다.
- Codex: `~/.codex/hooks.json`의 `hooks`에 `SessionEnd`, `PreCompact` 항목을 추가합니다. Codex에서 `/hooks`로 새 훅을 신뢰로 표시해야 실행됩니다.
- 압축 직전 훅은 세션이 끝나지 않아도 대화가 길어져 자동 압축(또는 수동 압축)이 일어날 때 그때까지의 대화를 큐에 넣습니다. 이미 기록한 부분은 다시 기록하지 않고, 다음 압축이나 세션 종료 때 이어서 기록합니다.
- `SessionEnd`만 연결된 이전 연결은 "Not connected"으로 보입니다. 다시 "Connect"을 누르면 빠진 `PreCompact`만 더합니다.

실행 파일명이 Windows에서는 `lore-cli.exe`, macOS·Linux에서는 `lore-cli`입니다. Unix 훅 경로는 셸 특수문자를 포함해도 리터럴로 실행되도록 인용합니다.

기존 훅과 다른 설정 키는 순서까지 그대로 두고 배열 끝에 추가합니다. 다른 설치 위치를 가리키는 이전 lore 훅은 새 경로로 바꿉니다. 바꾸기 전 파일은 같은 폴더에 `<파일>.lore-backup-<시각>`으로 남깁니다. 기록 모델 프로세스에는 `LORE_RECORDER=1`을 넣으므로, 그 세션이 끝나도 훅은 등록하지 않습니다.

## 데이터 위치

아래 `%LOCALAPPDATA%\io.github.junyoungjo.lore`는 Windows의 앱 데이터 위치입니다. macOS는 `~/Library/Application Support/io.github.junyoungjo.lore`, Linux는 `${XDG_DATA_HOME:-~/.local/share}/io.github.junyoungjo.lore`입니다. `LORE_HOME`으로 앱과 CLI의 데이터 폴더를 함께 재정의할 수 있습니다.

새 기억 저장소 기본값은 `~/Documents/doc/lore-memory`이고, 기존 `settings.json`에 지정한 위치는 유지합니다. 아래 `D:\doc\lore-memory`는 Windows에서 사용하던 위치의 예입니다. qmd 설정·색인·캐시는 기억 저장소가 아닌 앱 데이터의 `qmd/<기억 경로 해시>/` 아래에 PC별로 생성합니다. 처음 설정할 때 `index`를 실행하고 qmd MCP 연결의 환경 변수를 `doctor`가 보여 주는 경로에 맞춥니다. 기존 `.qmd/` 데이터는 자동으로 지우지 않습니다.

| 경로 | 내용 |
|---|---|
| `%LOCALAPPDATA%\io.github.junyoungjo.lore\settings.json` | 설정. 앱은 이 파일을 2초마다 확인해, 직접 고친 내용도 다시 켜지 않고 적용합니다. 처음 실행한 시각(`since`)은 수동 `scan` 명령이 그 이전 세션을 건너뛰는 기준입니다. |
| `%LOCALAPPDATA%\io.github.junyoungjo.lore\state.sqlite` | 작업 큐, 처리 위치, 실패 횟수, 최근 기록 |
| `%LOCALAPPDATA%\io.github.junyoungjo.lore\reviews\` | 최근 50개 처리의 모델 입력·응답 |
| `%LOCALAPPDATA%\io.github.junyoungjo.lore\migration\` | 링크 이전 계획과 백업. 훅 설정 백업은 변경한 설정 파일 옆에 별도로 남습니다. |
| `D:\doc\lore-memory\projects\<프로젝트>\`, `common\` | 기억 문서(qmd 색인 대상) |
| `D:\doc\lore-memory\sources\<claude|codex>\<세션 ID>.md` | 기억에 쓰인 세션의 대화 사본(qmd 색인 대상 아님) |

## 처리 규칙

- **실패 처리:** 실패한 작업은 다음 시도 시각을 정해 뒤로 미루므로 다른 작업을 막지 않습니다. 실행 실패·시간 초과는 1분부터 두 배씩 최대 1시간 간격으로 8번까지, 모델 응답 검사 실패는 3번까지 다시 시도합니다. 원문이 없거나 처리한 앞부분이 바뀐 경우처럼 다시 해도 같은 결과인 실패는 바로 `실패`로 둡니다. `실패`인 작업은 팝오버의 "Retry"로 다시 넣거나 "Skip"로 넘깁니다.
- **버리기:** 팝오버의 "Skip"를 누르고 한 번 더 확인하면, 실패한 작업의 처리 위치 다음 구간(지금 설정의 "New conversation per request" 기준으로 다시 계산)을 기록하지 않고 처리 위치를 그 구간 끝으로 옮깁니다. 남은 줄이 있으면 대기로 돌아가 다음 구간부터 이어서 기록하고, 없으면 완료로 둡니다. 원문 파일이 없거나, 처리한 앞부분이 바뀌었거나, 원문 JSON이 깨져 구간을 계산할 수 없으면 그 세션을 `건너뜀`으로 두어 더 기록하지 않습니다. 압축된 원문을 풀지 못한 경우처럼 다시 하면 될 수 있는 오류이면 아무것도 바꾸지 않고 오류를 보여 줍니다. 되돌릴 수 없습니다.
- **큐 화면:** 팝오버의 "Pending", "Retrying", "Failed" 숫자를 누르면 큐 화면에서 기록 중, 대기, 재시도 대기, 실패인 작업을 프로젝트, 세션 ID, 상태와 함께 보여 주고 누른 묶음으로 스크롤합니다. 아직 처리를 시작하지 않은 작업은 원문 앞부분에서 세션 ID와 작업 폴더를 읽어 프로젝트를 표시합니다. 지금 기록 중인 작업은 "Pending" 수에 넣지 않습니다.
- **긴 발언:** 12,000자를 넘는 발언은 앞 3/4과 뒤 1/4만 모델에 보내고 가운데를 `[… 중략 …]`으로 표시합니다. 인용문은 원래 발언 전체와 대조합니다.
- **기록하지 않는 세션:** Codex 하위 에이전트 세션(세션 파일의 `subagent` 표시), Claude `subagents` 폴더의 세션, git 저장소 밖에서 진행한 세션은 `건너뜀`으로 둡니다.
- **에이전트가 보낸 입력:** Orca나 다른 에이전트가 보내는 고정 형식 메시지는 입력마다 `agent`로 표시합니다. 대상은 "You are working inside Orca", "You are a dispatched worker", "Please carry out this task from my Orca coordinator", "Continue work from the prior Orca session", "The prior provider session is read-only context", "Original agent: ", "Another Claude session sent a message"로 시작하거나 첫 줄이 "You have N orchestration message(s)"인 입력입니다. 새로 들어온 구간의 입력이 모두 `agent`이면(입력 없이 답변만 있는 구간은 직전 입력이 `agent`이면) 기록 모델을 부르지 않고 넘어갑니다. 그 밖의 입력이 하나라도 있으면 구간 전체를 기록 모델에 보내고, 어느 입력이 사람이 직접 친 말인지와 무엇을 기록할지는 기록 모델이 판단합니다. `agent` 입력은 사용자 결정의 근거로 쓸 수 없습니다.
- **작업 도중 보낸 말:** 어시스턴트가 일하는 도중에 사용자가 보낸 메시지는 Claude 세션 파일에 `attachment` 기록(`queued_command`, `origin.kind: "human"`)으로 저장됩니다. 앱은 이것을 그 위치의 사용자 발언으로 읽습니다. 같은 형식으로 들어오는 하위 에이전트 보고(`origin.kind: "peer"`)는 `agent`로 표시하고, 작업 알림은 읽지 않습니다.
- **발언 시각:** 기록 모델에는 발언 시각을 이 PC의 현지 시각과 UTC 차이로 넘깁니다(예: `2026-11-04 16:50:45 +09:00`). 대화 사본의 발언 제목에는 원래대로 UTC로 적습니다.
- **프로젝트 ID:** 저장소 루트의 선택 파일 `.lore.json`에 `{"project":"고정-id"}`를 커밋하면 PC마다 체크아웃 폴더 이름이 달라도 같은 기억을 사용합니다. 기존 이름을 ID로 쓰면 기존 기억 경로를 유지합니다. 파일이 없으면 종전대로 저장소 폴더 이름을 사용합니다. 잘못된 ID 파일은 폴더명으로 우회하지 않고 실패로 표시합니다.
- **새 프로젝트:** 기억 폴더가 없는 저장소의 첫 기록이면 `projects/<이름>/`을 만들고 qmd 컬렉션 `project-<이름>`을 추가합니다.
- **기존 문서 입력:** 해당 프로젝트와 common 문서가 60,000자 이하이면 모두 보냅니다. 넘으면 qmd 벡터 검색으로 관련 문서를 골라 한도까지 본문을 보내고, 나머지는 경로와 제목만 보냅니다. 모델이 본문 없는 문서를 고쳐야 하면 `needs_notes`로 요청하고, 앱이 그 본문을 넣어 한 번 더 호출합니다. 본문을 받지 않은 기존 문서를 고치는 변경안은 거부합니다.
- **기존 문서 수정:** 기록 모델은 기존 문서의 본문 전체를 다시 쓰지 않고 고칠 곳만 보냅니다. 종류는 바꾸기(본문의 글을 그대로 옮긴 `find`를 새 글로 바꿈), 절 끝에 덧붙이기, 새 절 넣기입니다. 바꿀 글과 절 제목은 문서 본문에 정확히 한 번 있어야 하고, 그렇지 않으면 변경안을 거부하고 다시 시도합니다. 손대지 않은 글은 그대로 남습니다. 새 문서만 본문 전체를 씁니다. 기존 문서의 머리 정보(frontmatter)는 그대로 두고 `authority`만 바꾸며, `kind`는 없을 때만 넣습니다. 본문 첫 줄이 제목과 같은 `# ` 줄이면 지웁니다.
- **사본과 링크:** 새 근거 링크는 기억 문서 기준 상대 경로와 원본 발언 번호 앵커를 사용합니다(예: `../../sources/claude/<세션>.md#L42`). 대화 사본에 해당 앵커를 보존하므로 PC 이동이나 앞쪽 발언 추가에도 새 링크는 유지됩니다. 기존 절대 경로 링크는 `portable-links`로 미리 보고 이전할 수 있습니다. 이전 형식의 링크도 계속 지원하며, 사본을 다시 쓸 때 앞쪽에 발언이 새로 들어가 줄 번호가 밀리면, 그 사본을 가리키는 모든 기억 문서의 링크를 원본 줄 번호(`L<번호>`)로 맞춰 새 줄로 옮깁니다. 기록 모델이 고칠 곳으로 보낸 글 안의 링크에도 같은 대응을 적용합니다.
- **반영:** 검토 단계 없이 자동 반영합니다. 변경안 작성 이후 누가 문서를 고쳤으면 반영하지 않고 실패로 둡니다.
- **검토와 커밋:** 기억 저장소가 git 저장소(`<저장소>/.git`이 있음)이면, 상태 화면의 "To review" 칸에 마지막 커밋 이후 바뀐 기억 문서(`common/`, `projects/`) 수를 보여 줍니다. 칸을 누르면 검토 창이 열립니다. 검토 창은 바뀐 문서를 프로젝트별로 묶어(common이 먼저, 그다음 프로젝트 이름순) 그 문서를 고친 세션과 함께 보여 주고, 문서마다 마지막 커밋(왼쪽)과 지금(오른쪽)을 줄 단위로 나란히 보여 줍니다. 바뀐 줄 안에서는 바뀐 단어를 표시하고, 6줄을 넘게 이어지는 같은 줄은 앞뒤 2줄만 남기고 접습니다. 문서마다 체크박스가 있고, 창을 열 때는 모두 해제되어 있습니다. 프로젝트 머리글과 목록 맨 위의 체크박스로 묶음 전체를 고를 수 있습니다. 커밋하면 체크한 문서만 커밋하고, 체크하지 않은 문서는 다음 검토로 남습니다. 커밋 메시지 기본값은 "기억 기록 검토 (오늘 날짜)" 한 줄이고, 칸에서 고친 그대로 커밋합니다. 대화 사본 같은 다른 파일은 목록에 넣지 않고 커밋할 때마다 함께 커밋합니다. 다른 곳에서 stage해 둔 파일은 커밋에 넣지 않습니다. 검토 창이 열려 있는 동안에는 새 기록을 시작하지 않고, 열 때 기록 중이면 그 기록이 끝난 뒤 diff를 보여 줍니다. 창을 닫으면 다시 기록합니다. 이 로컬 검토 창에서 문서를 직접 고치는 일은 앱 밖에서 합니다. PC 사이 동기화의 선택안과 문구 정리는 별도 [동기화 검토 창](sync-review.md)에서 진행합니다.
- **최근 기록:** 상태 화면에는 앱 상태 DB에 남긴 최근 기록(최대 500개) 가운데 최근 30개를 보여 줍니다. 경과 시간과 "몇 분 전"은 15초마다 다시 계산합니다. "Recent activity" 옆의 "Clear"를 누르고 한 번 더 확인하면 최근 기록을 모두 지웁니다. 기억 문서, 큐, 실패한 작업은 그대로 둡니다.
- **처리 한도:** 설정 화면에서 기록 모델 시간 제한(기본 600초), qmd 색인 시간 제한(기본 900초), 한 번에 보내는 새 대화(기본 30,000자), 한 번에 보내는 기존 문서(기본 60,000자)를 바꿉니다. 시간 제한은 저장하면 실행 중인 호출에도 바로 적용되고, 글자 수 한도는 다음 구간부터 적용됩니다.

## CLI

```powershell
lore-cli doctor            # 현재 PC의 경로와 의존성 진단 (설정·DB를 만들지 않음)
lore-cli index             # common/과 기존 프로젝트 컬렉션 등록, 로컬 qmd 색인 갱신
lore-cli project           # 현재 저장소의 기억 프로젝트 ID 확인
lore-cli portable-links    # 기존 절대 경로 근거 이전 미리보기
lore-cli portable-links --apply  # 백업·중단 복구를 갖춘 명시적 이전
lore-cli status            # 큐 상태와 실패한 작업
lore-cli sync status       # 동기화 상태와 보존된 검토 자료
lore-cli sync check        # upstream 확인, 한쪽만 앞선 이력은 동기화
lore-cli sync analyze      # 양쪽 변경을 실제 기록 모델로 비교
lore-cli retry <작업 ID>    # 실패·건너뜀 작업을 다시 대기열에
lore-cli discard <작업 ID>  # 실패한 작업의 실패한 구간을 기록하지 않고 넘김(팝오버의 "Skip")
lore-cli enqueue claude <원문 경로>
lore-cli scan              # 훅이 놓친 세션을 직접 큐에 등록(앱은 자동으로 하지 않음)
lore-cli work [--once]     # 트레이 앱이 꺼져 있을 때 수동 처리
lore-cli hooks             # 훅 연결 상태 출력
lore-cli connect claude    # 전역 훅 설정에 연결(이전 파일 백업)
lore-cli disconnect claude # 전역 훅 설정에서 lore 훅 제거(이전 파일 백업)
lore-cli migrate-links [--apply] [--backup <폴더>]   # 원본 세션 링크를 대화 사본 링크로 이전
lore-cli refresh-copies [--apply]   # 큐에 있는 세션의 대화 사본을 지금의 읽기 방식으로 다시 만들고 링크를 옮김
```

`work`, `sync`, `index`, `portable-links`, `refresh-copies`는 트레이 앱이 실행 중이면 거부합니다(같은 잠금 파일을 씁니다).

## 이전 Python 프로토타입

`memory.py`, `runtime.py`, `tests/test_memory.py`, `build.ps1`, `standalone/`, `work/`, `work-auto/`, `examples/`, 루트의 `recorder-prompt.md`는 작업 스케줄러로 실행하던 이전 프로토타입입니다. 새 앱은 이 파일을 쓰지 않습니다. 검증 기록은 [VALIDATION.md](../VALIDATION.md)에 있습니다.
