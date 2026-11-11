# 여러 PC에서 사용하기

## 플랫폼과 파일 위치

| 항목 | Windows | macOS | Linux |
|---|---|---|---|
| 앱 데이터 | `%LOCALAPPDATA%\io.github.junyoungjo.lore` | `~/Library/Application Support/io.github.junyoungjo.lore` | `$XDG_DATA_HOME/io.github.junyoungjo.lore` 또는 `~/.local/share/io.github.junyoungjo.lore` |
| 앱 실행 파일 | `lore.exe` | `lore` | `lore` |
| 훅 CLI | `lore-cli.exe` | `lore-cli` | `lore-cli` |
| 설치 형식 | NSIS | app, dmg | deb, AppImage |
| 팝오버 | 화면 오른쪽 아래 | 메뉴 막대 아래 오른쪽 | 트레이 메뉴의 ‘열기’ |

Tauri의 [플랫폼 설정 병합](https://v2.tauri.app/develop/configuration-files/)을 사용합니다. Linux는 [트레이 클릭 이벤트를 지원하지 않기 때문에](https://v2.tauri.app/learn/system-tray/) 메뉴의 ‘열기’를 사용합니다. Linux 데스크톱에 트레이/AppIndicator 지원이 필요합니다.

`LORE_HOME`은 앱 데이터 폴더 전체를 바꾸는 선택 환경 변수입니다. 격리 시험이나 비표준 설치에 사용합니다. 앱에만 이 변수를 주고 Claude·Codex에는 주지 않으면 서로 다른 큐를 사용하므로, 실제 사용에서는 훅을 호출하는 프로세스에도 같은 값을 설정해야 합니다.

macOS Finder·로그인 실행에서는 터미널의 PATH가 전달되지 않을 수 있습니다. 기본 PATH 외에 `~/.local/bin`, `~/.bun/bin`, `~/.cargo/bin`, `~/.npm-global/bin`, Homebrew와 nvm의 Node 설치 디렉터리를 찾습니다. 그 밖의 설치 방식은 로컬 `settings.json`의 `commands` 배열에 실행 파일과 인수를 지정합니다. 셸 초기화 파일을 실행해서 경로를 얻지는 않습니다.

## 기존 Windows 기억 가져오기

1. 앱 소스와 기억 저장소를 각각 옮깁니다. Git bundle에 포함된 소스만으로 `D:\doc\lore-memory`나 `%LOCALAPPDATA%`의 운영 데이터가 복원되지는 않습니다.
2. 기억 저장소의 `common/`, `projects/`, `sources/`와 Git 이력을 함께 가져옵니다. `sources/`에는 근거 대화가 있으므로 문서만 가져오면 근거를 확인할 수 없습니다.
3. 새 PC의 `settings.json`에 그 PC의 기억 저장소 **절대 경로**를 지정합니다. 새 설치 기본값은 `~/Documents/lore-memory`입니다. Windows 경로를 Mac에 그대로 지정하면 거부합니다.
4. `lore-cli doctor`로 Codex·Claude·qmd 탐색 결과와 qmd 설정 위치를 확인합니다. 이 명령은 설정·DB·훅을 변경하지 않습니다. 설치된 CLI가 PATH에 없으면 설치 폴더의 실행 파일을 절대 경로로 실행합니다. macOS 앱 안의 CLI는 `Lore.app/Contents/MacOS/lore-cli`입니다.
5. 트레이 앱을 끈 상태에서 `lore-cli portable-links`로 이전 계획을 봅니다. `--apply`를 붙이면 이전합니다. 원본 세션 파일이나 Windows 드라이브가 없어도 가져온 대화 사본으로 정확한 발언을 찾습니다. 사본이나 정확한 줄을 찾을 수 없으면 보고하고 그 사본의 링크는 모두 보존합니다.
6. `lore-cli index`로 새 PC의 qmd 컬렉션과 색인을 만듭니다. `common`과 가져온 모든 유효한 프로젝트 폴더를 등록합니다. 실제 qmd 설치와 모델 다운로드가 필요한 단계입니다.
7. Codex·Claude가 사용하는 qmd MCP의 `QMD_CONFIG_DIR`, `XDG_CACHE_HOME`을 `doctor`의 `qmdConfigDir`, `qmdCacheDir`에 맞춥니다. 인덱스 이름도 `qmdIndex`와 맞춥니다. `.qmd/config`를 가리키던 이전 연결은 갱신해야 새 기록을 검색합니다.
8. 앱을 실행하고 원하는 제공자의 훅을 명시적으로 연결합니다. Codex의 신뢰 설정도 확인합니다.

로컬 큐 DB는 다른 PC로 공유하지 않습니다. 큐에는 원문 절대 경로와 처리 위치가 들어 있으므로 이 버전은 Windows 큐의 미처리 작업을 Mac 경로로 변환하지 않습니다. Windows에 남은 작업은 그 PC에서 처리하거나, 원문과 작업 저장소 경로를 별도로 준비해야 합니다.

## 고정 프로젝트 ID

각 소스 저장소 루트에 다음 파일을 커밋합니다.

```json
{"project":"demoengine"}
```

이름은 소문자 영문·숫자로 시작하고 영문·숫자·`-`·`_`만 쓰는 100자 이하 ID입니다. 기존 `projects/demoengine/` 기억을 계속 쓰려면 같은 ID를 지정합니다. 서로 다른 프로젝트에는 서로 다른 ID를 정합니다. 앱이 ID의 전역 유일성을 자동으로 판단하지는 않습니다.

파일이 없으면 기존 폴더명 식별을 유지합니다. 잘못된 ID 파일은 기록 실패로 표시합니다. worktree는 원래 저장소의 ID를 사용합니다. `lore-cli project`로 현재 저장소가 사용할 ID를 확인할 수 있습니다. 기존 기억 폴더의 자동 재명명·병합은 하지 않습니다.

## 근거 링크와 이전 복구

새 링크는 문서 기준 상대 경로와 원본 발언 번호입니다.

```markdown
[L42](<../../sources/claude/session-id.md#L42>)
```

`sources/`는 qmd 색인 대상이 아닙니다. 후속 에이전트가 `qmd://project-demoengine/topic.md`를 읽으면 실제 문서는 `<knowledgeRoot>/projects/demoengine/topic.md`이고, 근거의 상대 경로는 이 위치를 기준으로 풉니다. `#L42`는 사본의 `<a id="L42"></a>` 앵커이며 물리적 파일의 42번째 줄이라는 뜻이 아닙니다.

이전은 `common/`, `projects/`와 루트 `README.md`에 있는 대화 사본 링크를 대상으로 합니다. 다른 파일을 직접 작성해 절대 경로 링크를 넣었다면 별도로 확인해야 합니다. 원본 jsonl을 직접 가리키는 아주 오래된 링크는 기존 `migrate-links` 명령의 대상입니다. 먼저 사본으로 옮긴 뒤 `portable-links`를 실행합니다.

적용 전 원본·변경 후 내용 전체를 앱 데이터의 `migration/portable-links.pending.json`에 저장합니다. 중간에 중단돼도 같은 명령을 다시 실행하면 원본 또는 자체 적용 결과와 일치하는 파일만 이어서 처리합니다. 외부 수정이 있으면 덮어쓰지 않습니다. 완료 후 계획은 `portable-links-<시각>.backup.json`으로 남습니다. 이전 뒤에는 `index`로 문서 검색을 갱신합니다.

## 공유 범위와 동시 기록

Git으로 공유할 대상은 기억 Markdown과 대화 사본입니다. 설정·SQLite 큐·처리 이력·모델 입력/응답·qmd 설정과 캐시는 PC별 앱 데이터에 둡니다. 기존 기억 저장소의 `.qmd/`는 더 이상 새 색인의 위치로 사용하지 않으며 자동 삭제하지 않습니다.

기억 저장소에 remote와 upstream을 연결하면 앱이 기록을 커밋하고 다른 PC의 커밋과 병합해 보냅니다. **Auto-sync**는 기본 꺼짐이며, 켜면 기록이 끝난 직후와 5분마다 동기화합니다. 양쪽에 새 커밋이 있어도 사용자에게 묻지 않고 병합합니다. 두 PC가 같은 줄을 고쳤으면 양쪽 줄을 모두 남기고 그 노트를 확인할 항목에 올리며, 두 PC가 모두 고친 노트는 동기화 뒤에 모델로 비교합니다. 동작 순서와 실패 처리는 [동기화 안내](sync-review.md)에 있습니다.

분산 잠금은 없습니다. 전송 중 다른 PC가 먼저 push하면 그 커밋을 가져와 다시 병합한 뒤 보내며, 강제 push는 하지 않습니다. 검토 상태(`refs/lore/reviewed`)와 확인할 항목은 PC별로 따로 관리합니다.

## 검증 범위

CI에는 Windows·macOS·Linux의 Rust 검사와 화면 검사, 네이티브 앱 빌드를 구성했습니다. CI를 추가한 사실은 세 플랫폼에서 실제 실행이 완료됐다는 뜻이 아닙니다. 실제 훅 이벤트·알림·로그인 자동 실행·qmd 검색은 각 OS의 실제 설치 환경에서 확인해야 합니다.
