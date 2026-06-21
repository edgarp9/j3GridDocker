# j3GridDocker 도메인 명세

## 1. 목적

j3GridDocker는 하나의 관리 윈도우 내부 영역을 splitter 기반 격자로 나누고, 각 영역에 외부 프로그램의 top-level window를 배치해 함께 이동하고 크기를 맞추는 Windows/Linux 데스크톱 프로그램이다.

Windows UI는 Rust와 `windows-sys` 기반 Win32 entry로 구현한다. Linux UI는 GTK4 entry로 구현하며, 외부 창 제어는 X11 세션에서 `x11rb`를 통해 수행한다. 외부 프로그램 윈도우를 j3GridDocker의 자식 창으로 만들지 않고, OS 기준 독립된 top-level window 상태를 유지한 채 플랫폼별 window controller가 위치, 크기, 표시 여부만 제어한다.

Wayland 세션에서는 보안 모델상 다른 클라이언트 창의 전역 위치 조회, 이동, 숨김, 표시 제어가 허용되지 않는다. 따라서 Linux에서 Windows와 같은 외부 창 Dock 동작은 X11 세션을 전제로 하며, Wayland에서는 GTK4 UI와 내부 workspace 조작은 사용할 수 있지만 외부 창 제어 기능은 명확한 오류로 거부된다.

## 2. 범위

### 포함

- 여러 탭 관리
- 탭별 독립 레이아웃 저장
- 탭별 외부 프로그램 윈도우 배치
- active tab 안에서 이미 배치된 외부 프로그램 윈도우를 다른 빈 영역으로 재배치
- active tab 안에서 이미 배치된 외부 프로그램 윈도우를 j3GridDocker 밖으로 드롭하면 현재 위치에서 배치 해제
- splitter 기반 영역 분할과 비율 조정
- 외부 윈도우 드롭 감지 및 영역 배치
- 활성 탭 외부 윈도우 표시 및 위치/크기 동기화
- 비활성 탭 외부 윈도우 숨김
- 영역, 탭, 프로그램 종료 시 배치 해제와 상태 복원

### 제외

- 외부 윈도우를 j3GridDocker의 child window로 편입하는 방식
- 외부 윈도우를 시스템 전체 TopMost 창으로 만드는 방식
- 비활성 탭 전환을 위해 외부 윈도우를 최소화하는 방식
- 고정 행/열 테이블만으로 레이아웃을 표현하는 방식

## 3. 핵심 용어

### j3GridDocker Window

사용자가 직접 조작하는 메인 윈도우다. 탭, 영역, splitter, 영역 메뉴, 해제 버튼, tab preset 저장/불러오기/편집/삭제 메뉴, 전역 Options 메뉴, Help/About 메뉴를 포함한다.
탭바 왼쪽에는 탭 목록과 독립적으로 고정된 Show/Hide 버튼과 그 오른쪽의 New 버튼을 둔다. Show/Hide 버튼은 탭바 아래 j3GridDocker 작업 영역 UI의 표시 여부를 전환하고, New 버튼은 새 탭을 생성한다.
최상위 메뉴는 `Workspace`, `Layout`, `Presets`, `View`, `Options`, `Window`, `Help`로 구성한다. `Workspace`에는 탭 생성, 이름 변경, 닫기, 다른 탭 닫기 진입점을 둔다. `Layout`에는 선택 영역 분할/삭제와 선택 창 배치 해제 진입점을 둔다. `Presets`에는 tab preset 저장/불러오기/편집/삭제 진입점을 둔다. `View`에는 작업 영역 컨트롤 표시/숨김을 둔다. `Options`에는 `Dock While Workspace Controls Are Hidden` 토글, 항상 `Language`로 표시되는 UI 언어 선택 메뉴(`English`, `Korean`)를 둔다. `Window`에는 최소화, 최대화 또는 복원, 창 닫기를 둔다. `Help`에는 `About j3GridDocker`를 두며, About 창 제목은 `About j3GridDocker`로 표시한다. About 창은 상단에 `j3GridDocker {version}` 버전 라벨을 두고, 본문에는 빌드에 포함된 `about.txt` 원문을 읽기 전용 스크롤 영역으로 표시하며, 하단 GitHub 링크와 OK 버튼을 제공한다.
최상위 메뉴는 마우스 클릭뿐 아니라 Windows native menu와 같은 키보드 진입을 제공한다. main window에 포커스가 있을 때 `F10` 또는 단독 `Alt`로 첫 메뉴에 포커스를 둘 수 있고, 이후 Enter/Space, 아래 방향키, 좌우 이동, Escape 닫기 또는 플랫폼 기본 키보드 조작으로 메뉴 항목을 탐색하거나 실행할 수 있어야 한다. 최상위 메뉴 간 키보드 전환 중에는 한 번에 하나의 메뉴 popup만 표시한다.
`Dock While Workspace Controls Are Hidden` 옵션은 기본적으로 꺼져 있으며, 켜져 있으면 작업 영역 UI 숨김 상태에서도 보이지 않는 active tab layout bounds를 기준으로 external window drop을 Dock 후보로 판정한다. UI 언어 기본값은 영어이며, 사용자가 선택한 언어는 설정 파일에 저장해 다음 실행에서도 유지한다. 시작 시에는 저장된 tab 목록, active tab, tab별 splitter layout을 런타임 workspace로 복원하지 않고 새 워크스페이스로 시작한다. 이 경우에도 tab preset 목록과 UI 옵션은 설정 파일에서 로드한다. 숨김 상태에서도 Show/Hide 버튼을 오른쪽 클릭해 같은 옵션 메뉴를 열 수 있다.
탭 목록 폭이 client width를 초과하면 탭바 오른쪽에 overflow dropdown 버튼을 표시한다. 이 버튼은 숨겨진 탭 목록을 열어 탭 전환 진입점만 제공하며, 탭 overflow 위치는 저장하지 않는 런타임 UI 상태다.
Windows에서 기본 창 닫기 단축키인 `Alt+F4`는 `Window > Close Window`와 같은 종료 흐름을 실행한다. Linux GTK entry도 window manager 전달 여부에만 의존하지 않고 같은 단축키를 받아 동일한 close request 흐름으로 연결해야 한다.

### External Window

j3GridDocker가 배치 대상으로 관리하는 외부 프로그램의 top-level window다. HWND로 식별한다.

### Tab

서로 독립된 splitter layout과 external window placement를 가지는 작업 공간이다. 하나의 시점에는 하나의 active tab만 존재한다.

### Active Tab

현재 선택된 탭이다. active tab에 배치된 external window는 j3GridDocker의 현재 client area 기준으로 계산된 영역 좌표에 표시된다.

### Inactive Tab

현재 선택되지 않은 탭이다. inactive tab에 배치된 external window는 기본적으로 `ShowWindow(hwnd, SW_HIDE)`로 숨긴다.

### Region

splitter layout tree의 leaf node가 나타내는 배치 가능 영역이다. 하나의 region에는 최대 하나의 external window를 배치한다.
entry UI에 표시하는 region 제목은 현재 UI 언어를 따른다. 영어는 `Region N`, 배치된 경우 `Region N - docked`를 사용하고, 한국어는 `영역 N`, 배치된 경우 `영역 N - 배치됨`을 사용한다.
region 제목은 region rect에서 좌우 8px, 상하 6px을 제외한 영역 안에 단일 줄로 표시하며 세로 중앙 정렬과 끝줄임을 적용한다. 플랫폼별 entry는 현재 UI 언어의 문자를 실제 glyph로 렌더링할 수 있는 텍스트 렌더러를 사용해야 하며, Linux GTK drawing area에서는 Cairo toy text API가 아니라 Pango font fallback을 거치는 렌더링 경로를 사용한다.

### Split

하나의 region을 수평 또는 수직 방향으로 두 region으로 나누는 작업이다.

### Placement

특정 external window HWND를 특정 tab의 특정 region에 연결한 상태다. placement는 배치 전 상태를 함께 보관해야 한다.

### WindowSnapshot

external window를 region에 배치하기 전에 가능한 범위에서 저장한 복원용 상태다. HWND, window rect, 표시 상태, z-order 관련 참고 정보, style, ex-style을 포함할 수 있다.

### Undock

placement를 해제하고 external window를 가능한 범위에서 배치 전 상태로 복원하는 작업이다.

## 4. 설계 원칙

1. j3GridDocker는 external window의 부모를 변경하지 않는다.
2. external window는 OS 기준 top-level window로 유지한다.
3. j3GridDocker는 active tab에 속한 external window의 위치, 크기, 표시 여부만 제어한다.
4. "항상 보이도록 유지"는 시스템 전체 TopMost를 의미하지 않는다.
5. active tab에서 j3GridDocker가 정상 표시 상태이면 해당 external window를 저장된 region 좌표에 맞게 `ShowWindow`와 `SetWindowPos`로 표시한다.
6. inactive tab에 속한 external window는 `SW_HIDE`로 숨긴다.
7. inactive tab을 다시 active tab으로 전환하면 숨겨진 external window를 다시 표시하고 저장된 region 좌표에 맞게 재배치한다.
8. 비활성화를 위해 external window를 최소화하지 않는다.
9. j3GridDocker window가 minimized 상태가 되면 active tab의 external window를 `SW_HIDE`로 숨기고, restore 후 현재 client area 기준으로 다시 표시 및 재배치한다.
10. 모든 recoverable error는 `Result`로 전달한다.
11. 런타임 경로에서 `unwrap()`과 `expect()`를 사용하지 않는다.

## 5. 탭 명세

### 5.1 탭 속성

각 tab은 다음 상태를 가진다.

- 고유 ID
- 표시 이름
- splitter layout tree
- region별 placement
- 선택 여부

### 5.2 탭 추가

사용자는 새 tab을 추가할 수 있다.

새 tab은 기본적으로 하나의 root region을 가진다. 새 tab에는 external window가 배치되어 있지 않다.

빈 워크스페이스에서 생성하는 첫 tab의 기본 이름과 내부 `TabId`는 `0`부터 시작한다. 해당 tab의 root `RegionId`도 `0`부터 시작한다.

### 5.3 탭 삭제

사용자는 tab을 삭제할 수 있다.

custom-painted 탭바의 각 tab 오른쪽 닫기 버튼은 해당 tab 삭제 요청으로 처리한다. 닫기 버튼은 active tab과 inactive tab 모두에서 동작해야 하며, 탭 본문 hit-test와 닫기 버튼 hit-test는 서로 다른 의미로 분리한다. 탭 본문 클릭은 tab 전환이고, 닫기 버튼 클릭은 tab 삭제다.

탭 위에서 우클릭하면 해당 tab을 기준으로 context menu를 연다. 메뉴는 최소 `Rename tab`, `Close tab`, `Close other tabs`를 제공한다. tab preset 항목은 `Save tab preset...`과 `Load tab preset...`만 제공하며 `Edit tab preset...`, `Delete tab preset...`는 표시하지 않는다. `Close tab`은 context menu를 연 tab을 삭제하며, 대상 tab이 active tab이 아니어도 삭제할 수 있다.

탭바의 visible tab과 overflow dropdown이 아닌 빈 공간에서 우클릭하면 tabbar context menu를 연다. 메뉴는 최소 `New tab`을 제공하며, active tab이 있으면 `Rename active tab`, `Close active tab`, `Close other tabs`도 제공한다. 또한 title bar 버튼과 같은 의미의 `Minimize window`, `Maximize window` 또는 `Restore window`, `Close window`를 제공한다. active tab 대상 항목은 context menu를 연 순간의 active tab을 대상으로 고정하고, 기존 tab context menu와 같은 command 및 app 흐름을 사용한다. window 항목은 Win32 main window 상태 변경 및 `WM_CLOSE` 종료 흐름을 사용한다. tab context의 `Load tab preset...`는 바로 preset 이름을 중첩 표시하지 않고 명령을 먼저 선택한 뒤 context 대상 tab 아래 preset 목록 popup을 연다. tabbar context의 `Load/Edit/Delete ...` tab preset 항목은 tab bar 아래 preset 목록 popup을 열며, 저장된 preset이 없으면 현재 UI 언어 기준 status를 표시한다.

탭바의 visible tab과 overflow dropdown이 아닌 빈 공간에서 더블클릭하면 title bar의 maximize/restore 버튼과 같은 의미로 main window의 maximized 상태를 토글한다.

`Close other tabs`는 context menu를 연 tab을 남기고 나머지 tab을 삭제한다. 각 tab 삭제는 단일 tab 삭제와 같은 app 흐름을 사용하므로, 해당 tab의 placement는 기존 tab 삭제 정책에 따라 undock되고 실패 시 해당 tab 삭제 상태가 rollback된다. 일괄 삭제 중 일부 tab 삭제가 실패하면 best-effort 정책으로 남은 tab 삭제를 계속 시도한다. 최종 status bar는 기준 tab, 성공/전체 개수, 실패한 tab 작업, 현재 active tab, undock 집계를 표시한다.

tab 삭제 시 해당 tab에 배치된 모든 external window를 undock한다. 삭제 대상 tab이 inactive tab이라서 external window가 숨겨진 상태여도, undock 과정에서 다시 표시 가능한 상태로 복원해야 한다.

active tab을 삭제한 경우 남아 있는 tab 중 하나를 새 active tab으로 선택한다. 남은 tab이 없으면 새 tab을 자동 생성하지 않고 active tab이 없는 빈 상태를 허용한다.

마지막 tab 삭제로 워크스페이스가 비면 tab ID와 region ID 발급 카운터를 0으로 되돌린다. 이후 사용자가 새 tab을 만들면 기본 이름은 다시 `Tab 0`이 되고 root region도 `RegionId(0)`부터 시작한다.

탭 삭제 성공 또는 실패는 status bar에 표시한다. 성공 시 삭제된 tab, 현재 active tab 유무, undock 시도/복원/누락/실패 개수를 함께 보여준다.

### 5.4 탭 이름 변경

사용자는 tab의 표시 이름을 변경할 수 있다.

탭 이름은 UI 표시와 저장 상태 식별 보조 정보로 사용한다. 내부 참조는 이름이 아니라 고유 ID를 기준으로 한다.

초기 Win32 entry 구현은 tab context menu의 `Rename tab`에서 간단한 modal edit dialog를 띄워 이름을 입력받는다. 사용자가 dialog를 취소하면 tab 이름을 변경하지 않고 취소 status를 표시한다. 빈 이름 또는 공백뿐인 이름은 domain의 tab 이름 검증 오류로 처리하며, 기존 이름을 유지하고 어떤 tab 이름 변경이 실패했는지 status bar에 표시한다. modal input dialog는 OK 또는 Cancel 중 하나의 결과만 entry 흐름에 전달해야 하며, OK로 상태 변경을 적용한 뒤 close/destroy 이벤트가 추가 Cancel 결과를 발생시켜 성공 status를 덮어쓰면 안 된다.

### 5.5 탭 hover tooltip

사용자가 visible tab 본문 또는 닫기 버튼 영역 위에 마우스를 올리면, entry 계층은 해당 tab에 현재 dock된 external window title 목록을 tooltip으로 표시한다.

tooltip 대상은 active tab과 inactive tab을 구분하지 않고 해당 tab의 placement 전체를 기준으로 한다. title은 표시 시점에 가능한 범위에서 OS window title을 읽어 구성하며, title이 비어 있는 살아있는 window는 제목 없는 창으로 표시한다. dock된 window가 없거나 살아있는 window title을 하나도 확인할 수 없으면 해당 tab tooltip은 표시하지 않는다.

### 5.6 탭 전환

tab 전환은 다음 순서로 처리한다.

1. 선택된 tab이 존재하는지 확인한다.
2. 새 active tab 후보의 layout tree를 j3GridDocker client area 기준 region rect로 계산한다.
3. 새 active tab 후보의 각 placement에 대해 external window가 여전히 같은 HWND인지 확인한다.
4. stale HWND이면 해당 placement를 제거하고 계속 진행한다.
5. 유효한 external window를 `ShowWindow(hwnd, SW_SHOWNOACTIVATE)` 또는 `ShowWindow(hwnd, SW_SHOW)`로 표시한다.
6. 계산된 region rect에 맞게 `SetWindowPos`로 위치와 크기를 조정한다.
7. 현재 active tab의 모든 placement를 순회하며 external window가 여전히 같은 HWND인지 확인한다.
8. 이전 active tab의 placement가 stale HWND이면 해당 placement를 제거하고 숨김 호출 없이 계속 진행한다. 숨김 직전에는 유효했지만 `ShowWindow(hwnd, SW_HIDE)` 시점에 이미 종료된 경우도 stale HWND 제거로 처리한다.
9. 유효한 이전 active tab external window에 `ShowWindow(hwnd, SW_HIDE)`를 호출한다.
10. 선택된 tab을 active tab으로 변경한다.

`SW_SHOWNOACTIVATE`와 `SW_SHOW` 중 어떤 값을 사용할지는 포커스 정책에 따라 결정한다. 기본 정책은 탭 전환만으로 외부 프로그램 포커스를 강제로 빼앗지 않도록 `SW_SHOWNOACTIVATE`를 우선 검토한다.

탭 전환 실패 시 app 계층의 rollback/partial failure 정책을 entry 계층에서 왜곡하지 않는다. 대상 tab 표시 또는 배치 실패는 이전 active tab을 유지하고, 이미 표시한 대상 tab window는 숨김 rollback을 시도한다. 이전 active tab 숨김 실패도 이전 active tab을 유지하고 대상 tab window 숨김 rollback을 시도한다. 같은 active tab을 다시 선택한 경우에는 active tab을 그대로 유지한 상태에서 재표시 실패로 구분한다. status bar에는 가능한 경우 대상 tab 이름과 이전 active tab 이름을 포함하고, 내부 원인은 로그/에러 체인으로 남기되 사용자 문구는 “무엇이 실패했는지”와 “현재 어떤 tab이 유지되었는지”를 한국어로 분리해 표시한다.

stale HWND 제거 후 전환이 성공한 경우에는 성공 status에 제거된 대상 창 수와 이전 active tab 창 수를 구분해 포함한다. 예: `탭을 전환했습니다: Work (탭 2). 이전 활성 탭: Main (탭 1). 유효하지 않은 대상 창 1개를 배치에서 제거했습니다. 유효하지 않은 이전 활성 탭 창 1개를 배치에서 제거했습니다`.

활성 tab 삭제 후 자동 전환이 실패하면 삭제를 rollback하고 이전 active tab을 유지한다. 이 경우 status bar는 단순 삭제 실패가 아니라 “탭 삭제 후 자동 전환 실패”로 표시하고, 삭제 대상 tab과 자동 전환 대상 tab을 가능한 범위에서 함께 표시한다.

### 5.7 탭 overflow

탭 전체 폭이 현재 client width를 초과하면 custom-painted 탭바 오른쪽에 overflow dropdown 버튼을 표시한다.

visible tab range는 고정 탭 폭과 현재 client width를 기준으로 entry 계층에서 계산한다. 각 visible tab rect는 client rect 밖으로 그려지지 않아야 하며, dropdown 버튼 영역과 겹치지 않아야 한다. 탭 hit-test와 닫기 버튼 hit-test는 visible tab rect에 대해서만 수행한다.
entry 계층은 고정 탭 폭 전체가 viewport 안에 들어오는 탭만 visible로 취급한다. 탭 전체 폭이 탭바 공간을 초과하면 overflow dropdown 폭과 dropdown gap을 먼저 예약한 뒤 visible count를 계산한다. viewport가 탭 1개 전체 폭보다 좁으면 visible count는 0이며, 모든 탭은 overflow dropdown 대상이 된다. 이 상태에서는 탭 조각을 그리거나 hit-test하지 않는다.

active tab은 탭 추가, 탭 전환, 탭 삭제, 창 resize 후 가능한 한 visible range 안에 들어오도록 first visible tab index를 자동 보정한다. overflow dropdown에서 숨겨진 탭을 선택하면 해당 탭으로 전환한 뒤 visible range를 다시 계산한다.

overflow first visible tab index는 저장 모델에 포함하지 않는다. 창 resize 또는 탭 목록 변경으로 overflow가 해소되면 visible range는 첫 탭부터 시작하는 단순 탭바 동작으로 되돌아간다.

overflow 조작 성공은 status bar에 별도로 과하게 표시하지 않는다. dropdown을 열 수 없거나 숨겨진 탭이 없는 경계 상황처럼 사용자가 이해해야 하는 경우에만 명확한 상태 메시지를 표시한다.

### 5.8 탭 재정렬

사용자는 작업 영역 UI가 표시된 상태에서 visible tab 본문을 마우스로 드래그해 tab 순서를 바꿀 수 있다.

탭 순서 변경은 tab 이름이나 현재 index가 아니라 고유 `TabId` 기준으로 처리한다. entry 계층은 포인터 시작점, threshold 판정, visible tab hit-test, insertion 위치 표시만 담당하고, 실제 순서 변경 규칙은 `Workspace` 또는 app 유스케이스가 수행한다.

짧은 클릭은 기존 tab 전환으로 처리한다. 마우스 이동량이 tab drag threshold 이상이면 tab 전환을 보류하고 reorder drag로 판정한다. 닫기 버튼과 overflow dropdown hit-test가 tab reorder보다 우선하며, 닫기 버튼 영역에서는 reorder drag를 시작하지 않는다.

작업 영역 UI 숨김 상태에서는 기존 window move 정책을 우선한다. 이 상태에서 tab 본문을 짧게 클릭하면 tab 전환으로 처리하고, threshold 이상 드래그하면 j3GridDocker window move로 처리한다. 따라서 UI 숨김 상태에서는 tab reorder를 시작하지 않는다. 닫기 버튼 hit-test는 본문 클릭보다 우선하므로 닫기 버튼 영역에서는 tab 전환, reorder drag, window move drag를 시작하지 않는다.

드래그 중에는 현재 insertion 위치를 tab strip 위에 시각적으로 표시한다. overflow 상태에서는 현재 visible range의 tab rect와 midpoint를 기준으로 insertion index를 계산한다. 포인터가 visible viewport 좌우 edge 근처에 있고 숨겨진 tab이 남아 있으면 entry 계층은 드래그 중 first visible tab index를 자동으로 이동해 숨겨진 tab도 순차적으로 재정렬 대상에 포함할 수 있다.

mouse capture를 잡은 상태에서 드래그를 추적하고, 정상 release에서는 마지막 insertion 위치로 순서 변경을 확정한다. capture 상실, cancel mode, minimize 등 취소 이벤트에서는 순서 변경을 커밋하지 않고 drag 상태와 insertion 표시를 정리한다. 창 밖에서 release되더라도 capture가 유지되어 있으면 마지막으로 계산된 insertion 위치를 기준으로 안전하게 완료한다.

순서 변경 후 active tab ID는 그대로 유지한다. 설정 저장 모델의 tab 목록 순서는 변경된 runtime tab 순서를 그대로 반영한다.

## 6. 레이아웃 명세

### 6.1 레이아웃 모델

격자 레이아웃은 고정 행/열 테이블이 아니라 splitter 기반 분할 트리로 표현한다.

layout node는 다음 두 종류다.

- `Region`: 배치 가능한 leaf node
- `Split`: 두 child node와 분할 방향, 비율을 가진 internal node

### 6.2 분할 방향

분할 방향은 다음 중 하나다.

- `Vertical`: 좌우 두 영역으로 나눈다.
- `Horizontal`: 상하 두 영역으로 나눈다.

### 6.3 비율 저장

splitter 위치는 픽셀이 아니라 비율로 저장한다.

비율은 부모 rect 기준 첫 번째 child가 차지하는 크기의 비율이다. 예를 들어 vertical split의 ratio가 `0.4`이면 첫 번째 child는 부모 너비의 40%, 두 번째 child는 나머지 60%를 차지한다.

비율은 최소 영역 크기를 보장하는 범위로 제한한다.
초기 구현의 기본 최소 영역 크기는 domain 상수 `DEFAULT_MIN_REGION_SIZE`로 분리하며, app 계층은 이 값을 설정값처럼 주입해 layout 계산에 사용할 수 있다.
splitter drag 등으로 픽셀 위치에서 비율을 계산할 때는 부모 rect의 해당 축 길이와 첫 번째 child 크기를 기준으로 `SplitRatio`를 만든다. 부모 크기가 너무 작거나, 첫 번째 또는 두 번째 child가 최소 영역 크기보다 작아지는 위치는 domain error로 처리한다.

### 6.4 영역 계산

j3GridDocker client area가 변경되면 active tab의 layout tree를 다시 계산한다.

계산 결과 각 region은 화면 좌표 기준 rect를 가진다. external window 배치에는 이 화면 좌표 rect를 사용한다.

### 6.5 영역 분할

사용자는 region을 vertical 또는 horizontal 방향으로 2분할할 수 있다.

분할된 기존 region의 placement 처리 정책은 다음 중 하나로 명확히 정한다.

- 기본 정책: 기존 placement는 첫 번째 child region으로 이동한다.
- 대안 정책: 분할 전에 사용자에게 placement 이동 대상 region을 선택하게 한다.

초기 구현은 기본 정책을 사용한다.
이를 위해 split 대상이던 기존 region ID를 첫 번째 child에 그대로 유지하고, 새 region ID는 두 번째 child에 부여한다. 따라서 기존 placement는 region ID 변경 없이 첫 번째 child에 남는다.
entry UI의 선택 영역도 Win32 기준으로 기존 region ID를 유지한다. 분할 직후 새 region을 자동 선택하지 않는다.

### 6.6 splitter 드래그

사용자는 splitter를 드래그해 인접 영역 비율을 조절할 수 있다.

드래그 중에는 active tab의 region rect를 다시 계산하고, 배치된 external window의 위치와 크기를 실시간 또는 throttled 방식으로 갱신한다.

작업 영역 UI가 표시된 상태이거나 `Dock While Workspace Controls Are Hidden` 옵션이 켜진 숨김 상태에서 Ctrl 키를 누르고 있으면 entry 계층은 현재 active tab의 splitter hit 영역 위에 짧게 표시되는 top-level overlay window를 배치한다. 이 overlay는 external window보다 먼저 마우스 down을 받아 같은 splitter drag 흐름으로 전달하기 위한 입력 표면이며, layout tree나 placement 모델에는 포함하지 않는다. Ctrl을 놓거나 splitter drag가 시작되거나 overlay 허용 조건이 해제되거나 main window가 minimized 상태가 되면 overlay를 숨긴다. Linux GTK4/X11 구현도 같은 정책을 따른다.

### 6.7 영역 삭제

사용자가 region을 삭제하면 해당 region에 배치된 external window는 undock한다.

삭제된 region의 sibling node는 부모 node 자리를 대체한다. 이때 layout tree의 불필요한 split node를 정리한다.
root region은 삭제할 수 없으며, 이 경우 domain error를 반환한다. 존재하지 않는 region 삭제 요청도 domain error로 반환한다.

### 6.8 탭 preset

탭 preset은 현재 tab의 splitter layout과 각 leaf region에 dock된 외부 프로그램 실행 정보를 함께 저장하는 이름 있는 템플릿이다. leaf마다 선택적으로 프로그램 정보를 가진다.

tab preset은 다음 상태를 가진다.

- canonical preset 이름
- `TabPresetNode` root
- 각 leaf의 optional `ExternalProgramSpec`

`ExternalProgramSpec`는 외부 프로그램을 다시 실행하기 위한 실행 파일 경로, 실행 arguments 목록, 사용자 확인을 위한 저장 시점 window title을 가진다. Win32에서 dock된 HWND를 저장할 때는 process image path만 자동으로 채우고 arguments는 빈 목록으로 둔다. 사용자는 저장된 tab preset 편집 기능으로 실행 파일 경로와 arguments를 수정할 수 있다. 작업 디렉터리는 저장하지 않는다.

`save_active_tab_preset(name)`은 active tab의 splitter layout을 region ID 없이 저장하고, 현재 dock된 placement가 있으면 각 placement의 region leaf에 프로그램 실행 정보를 연결한 임시 tab preset을 만든다. 실제 저장 전에 `edit_tab_preset(name)`과 같은 스크롤 가능한 단일 다이얼로그를 열어 tab preset 이름과 모든 프로그램의 실행 파일 경로 및 arguments를 함께 편집할 수 있게 한다. 이 다이얼로그에서 cancel하면 tab preset은 저장하지 않는다. 같은 이름의 tab preset은 canonical name 기준으로 덮어쓴다.

`apply_tab_preset_to_tab(name, target_tab)`은 preset root를 새 `LayoutNode`로 변환하면서 leaf마다 새 `RegionId`를 발급하고, 저장된 프로그램 정보를 새 region ID와 함께 반환한다. 적용이 성공하면 대상 tab 이름은 canonical preset 이름으로 변경한다. entry 계층은 반환된 프로그램을 실행하고 생성된 top-level window를 해당 region에 dock한다. 이미 dock된 window가 있는 대상 tab에 preset을 적용할 때는 기존 window를 먼저 undock한 뒤 적용한다. 실행 또는 dock 실패는 가능한 프로그램까지 계속 시도하고 status bar에 대상 tab, 기존 undock 수, 프로그램 dock/전체, 실패 수와 실패 상세를 표시한다.

`delete_tab_preset(name)`은 canonical name 기준으로 저장된 tab preset을 삭제한다. 삭제는 현재 tab layout이나 external window placement를 변경하지 않는다. 저장과 편집 성공 status는 preset 이름과 프로그램 수를 표시하고, 삭제 성공 status는 삭제한 preset 이름을 표시한다. 저장/편집 dialog cancel은 저장소를 변경하지 않고 cancel status를 표시한다.

`edit_tab_preset(name)`은 저장된 tab preset의 이름과 모든 프로그램 leaf를 스크롤 가능한 단일 다이얼로그에서 편집한다. 이 편집 다이얼로그는 사용자가 마우스로 크기를 조절할 수 있고, 크기가 바뀌면 프로그램 목록 영역과 하단 버튼을 다시 배치한다. tab preset 이름은 빈 문자열이나 NUL 문자를 허용하지 않고, 이름이 바뀌면 기존 preset 항목을 새 이름의 preset으로 교체한다. 실행 파일 경로는 빈 문자열이나 NUL 문자를 허용하지 않고, arguments는 shell 문자열이 아니라 실행 시 `Command::arg`로 전달할 문자열 목록으로 저장한다. UI 입력에서는 공백으로 arguments를 나누고 큰따옴표로 공백 포함 argument를 입력할 수 있다. 편집 dialog의 기본 버튼은 OK이며, 이름/경로/arguments 입력 칸에서 Enter를 누르면 Windows dialog처럼 OK 응답을 실행한다. 저장/편집/검증 message dialog도 OK 또는 Cancel 결과를 한 번만 소비해 성공 후 취소 callback이 다시 실행되지 않게 한다. 입력 검증 실패는 최종 응답으로 소비하지 않고, 검증 메시지를 닫은 뒤 같은 편집 dialog에서 다시 OK를 시도할 수 있다.

Win32 entry UI는 최상위 `Presets` 메뉴에 tab preset 명령을 둔다. 이 메뉴는 `Save Tab Preset...`, `Load Tab Preset`, `Edit Tab Preset`, `Delete Tab Preset`을 제공한다. 저장은 현재 active tab 또는 tab context target을 기준으로 수행하고, 저장용 편집창에서 tab preset 이름을 입력받는다. 불러오기는 대상 tab의 기존 placement를 먼저 undock한 뒤 layout과 프로그램 실행을 복원한다.

## 7. 외부 윈도우 배치 명세

### 7.1 배치 대상

배치 대상은 top-level external window HWND다.

다음 window는 배치 대상에서 제외한다.

- j3GridDocker 자신의 window
- 이미 같은 tab 또는 다른 tab에 배치된 window
- 유효하지 않은 HWND
- 표시/크기 제어가 불가능한 window
- OS 또는 보안 정책상 제어가 제한된 window

### 7.2 드래그 감지

사용자가 external window를 마우스로 이동하는 동안 j3GridDocker는 주기적으로 다음 값을 감지한다.

- 현재 cursor position
- cursor 아래 또는 이동 중인 대상 HWND
- 대상 HWND의 press 중 window rect 변화 여부
- 마우스 버튼 상태
- j3GridDocker region hit-test 결과

마우스 버튼을 놓은 위치가 active tab의 region 안이고, press 중 대상 external window의 window rect 변화가 관측된 경우에만 해당 region에 external window를 배치한다. 단순 클릭, 텍스트 선택, 버튼 조작처럼 external window 자체가 이동하지 않은 입력은 drop으로 처리하지 않는다.

작업 영역 UI 숨김 상태에서는 기본적으로 기존 정책을 유지해 region hit-test를 Dock 후보로 사용하지 않는다. `Dock while hidden` 옵션이 켜져 있으면 숨김 상태에서도 현재 active tab의 숨겨진 작업 영역 bounds를 기준으로 region hit-test를 수행하고, hit된 region에 Dock 또는 placement 이동을 수행한다.

### 7.3 배치 전 상태 저장

external window를 배치하기 전에 가능한 범위에서 다음 상태를 저장한다.

- HWND
- window rect
- 표시 상태
- top-level owner
- placement 이전 z-order 관련 참고 정보
- 복원에 필요한 window style 또는 ex-style

현재 명세에서는 external window의 부모를 변경하지 않지만, dock 중에는 top-level owner를 j3GridDocker window로 설정할 수 있으므로 원래 owner를 함께 저장한다. 다만 undock 또는 detach 시에는 저장된 owner로 되돌리지 않고 `GWLP_HWNDPARENT`를 `0`으로 설정해 owner 없음으로 해제한다. 단, 향후 구현 과정에서 style을 조정하는 경우 원래 style을 함께 저장하고 undock 시 복원해야 한다.

### 7.4 배치 처리

external window를 region에 배치할 때는 다음 순서로 처리한다.

1. HWND 유효성을 확인한다.
2. window가 배치 제외 대상인지 확인한다.
3. 새 배치와 충돌할 수 있는 기존 placement만 확인한다. 대상 region의 placement 또는 같은 HWND placement가 이미 종료되었거나 snapshot identity와 달라진 stale 상태라면 placement를 제거한다.
4. 여전히 live placement가 같은 region 또는 같은 HWND에 남아 있으면 초기 구현 정책에 따라 거부한다.
5. 배치 전 상태를 저장한다.
6. active tab의 region rect를 계산한다.
7. `ShowWindow`로 window를 표시 가능한 상태로 만든다.
8. `SetWindowPos`로 region rect에 맞게 위치와 크기를 조정한다.
9. tab placement 목록에 HWND와 region ID를 저장한다.

하나의 region에는 하나의 external window만 배치한다.

### 7.5 활성 탭 동기화

active tab에 배치된 external window는 다음 이벤트에서 region rect에 맞게 재배치한다.

- j3GridDocker window 이동
- j3GridDocker window 크기 변경
- j3GridDocker window minimized 상태에서 restore
- splitter ratio 변경
- region split 또는 delete
- tab 재선택
- external window가 임의로 이동 또는 크기 변경된 뒤 다시 보정이 필요한 경우

동기화는 `SetWindowPos`를 사용한다. active tab의 external window는 j3GridDocker의 child window가 아니므로 위치만 동기화하면 메인 윈도우 활성화/이동 과정에서 z-order가 메인 윈도우 뒤로 밀릴 수 있다. 따라서 active dock 위치 보정은 external window를 j3GridDocker의 owned top-level window로 연결한 뒤 위치/크기와 비-TopMost z-order를 갱신해 external window가 j3GridDocker content area 위에 보이도록 한다. 이 동작은 `SetParent` 또는 `HWND_TOPMOST`를 사용하지 않으며, 시스템 전체 항상 위 창으로 만들지 않는다. 배치 해제 시에는 `GWLP_HWNDPARENT`를 `0`으로 설정해 owner를 해제한다.
j3GridDocker window가 minimized 상태이면 active tab external window를 숨기고, minimized 중에는 위치 동기화를 수행하지 않는다. restore 또는 maximize 이후에는 `ShowWindow` 후 `SetWindowPos` 순서로 현재 region rect에 맞춘다.
active tab 재표시, tab 재선택, resize/move 동기화 중 HWND identity가 더 이상 같은 외부 창이 아니면 해당 placement를 제거하고, entry 계층의 occupied-region 표시 캐시는 즉시 무효화해 UI가 제거된 placement를 계속 점유 상태로 그리지 않게 한다.

### 7.6 비활성 탭 숨김

inactive tab에 속한 external window는 기본적으로 `ShowWindow(hwnd, SW_HIDE)`로 숨긴다.

숨겨진 external window는 tab이 다시 active tab이 되거나, tab 삭제/프로그램 종료로 undock될 때 다시 표시 가능한 상태로 복원한다.

## 8. 배치 해제 명세

### 8.1 해제 진입점

사용자는 다음 방식으로 placement를 해제할 수 있다.

- region context menu
- region 상단 해제 버튼
- active tab에서 배치된 external window를 선택한 뒤 해제 버튼
- tab 삭제
- region 삭제
- j3GridDocker 종료

### 8.2 해제 처리

undock은 다음 순서로 처리한다.

1. placement 정보를 찾는다.
2. HWND가 여전히 유효한지 확인한다.
3. inactive tab에 속해 숨겨진 window라면 표시 가능한 상태로 전환한다.
4. `SetWindowLongPtrW(GWLP_HWNDPARENT, 0)`으로 top-level owner를 해제한다.
5. 저장된 style 또는 ex-style이 있으면 가능한 범위에서 복원한다.
6. 저장된 window rect로 위치와 크기를 복원한다.
7. 저장된 표시 상태에 맞게 `ShowWindow`를 호출한다.
8. placement 목록에서 제거한다.

HWND가 이미 사라졌다면 placement만 제거하고 사용자에게 필요한 수준의 로그 또는 상태 메시지를 남긴다.

active tab의 external window는 j3GridDocker의 child window가 아니므로, 사용자가 docked window 자체를 클릭해도 j3GridDocker content region의 마우스 이벤트가 직접 발생하지 않는다. 따라서 entry 계층은 active tab에 이미 배치된 HWND를 새 placement 등록 대상과 구분하되, 기존 placement drag 후보로는 추적해야 한다. 이 HWND가 j3GridDocker의 owner window 관계 안에 있더라도 active placement이면 region 선택을 동기화하고, 이동 threshold 이후 내부 region 이동 또는 외부 drop detach 흐름으로 이어져야 한다. 이후 해제 버튼은 이 region의 placement를 undock한다.

### 8.3 복원 한계

Windows 보안 정책, 대상 프로세스 상태, DPI 변경, 모니터 구성 변경, 대상 window의 자체 크기 제한 때문에 완전한 복원이 불가능할 수 있다.

복원 실패는 j3GridDocker 전체 종료 실패로 이어지지 않아야 한다. 실패한 HWND와 원인을 내부 로그에 남긴다.

## 9. 종료 명세

j3GridDocker 종료 시 모든 tab의 모든 placement를 undock한다.

종료 시 inactive tab에 속해 `SW_HIDE` 상태인 external window도 다시 표시 가능한 상태로 복원해야 한다. 숨겨진 external window를 그대로 남기지 않는다.

종료 순서는 다음과 같다.

1. active tab과 inactive tab 전체 placement 목록을 수집한다.
2. 각 placement는 종료 직전 active/inactive 여부와 관계없이 먼저 표시 가능한 상태로 전환한 뒤 undock을 시도한다.
3. 실패한 항목은 로그로 기록하고 나머지 undock을 계속한다.
4. 모든 가능한 external window 복원이 끝난 뒤 j3GridDocker window를 종료한다.

종료 직전에는 active tab의 external window도 j3GridDocker window minimize 등으로 `SW_HIDE` 상태일 수 있다. 따라서 종료 undock은 inactive tab만 특별 취급하지 않고 모든 runtime placement에 대해 표시 복원을 먼저 시도한다. 이 동작은 현재 실행 중 placement 정리를 위한 것이며, 저장된 HWND를 다음 실행에서 자동 재연결하는 정책을 의미하지 않는다.

## 10. 저장 모델

초기 구현에서 저장해야 하는 영속 상태는 다음과 같다.

- tab 목록
- active tab ID
- tab 표시 이름
- tab별 layout tree
- split 방향과 ratio
- region ID
- region별 배치 정보
- tab preset 목록

구현된 설정 모델은 domain의 `WorkspaceSettings`, `TabSettings`, `SavedPlacement` 값 객체로 표현한다. 설정 파일 I/O와 TOML DTO 변환은 infra의 `SettingsFileStore`가 담당한다. 기본 저장 위치는 프로그램 실행 파일과 같은 폴더이며, 파일 이름은 실행 파일 이름의 확장자를 `.toml`로 바꾼 값이다. 예를 들어 `j3grid-docker.exe`로 실행되면 같은 폴더의 `j3grid-docker.toml`을 사용한다.

`SavedPlacement`는 region ID, 마지막으로 관측한 HWND, 배치 전 `WindowSnapshot`, 복원 정책을 저장한다. HWND는 실행 중에만 안정적으로 의미가 있으므로 저장된 HWND를 다음 실행에서 live placement로 자동 등록하지 않는다. 설정 로드 시 저장된 tab 목록, active tab ID, tab 이름, layout tree, split 방향/ratio, region ID는 런타임 workspace에 복원하지 않는다. 저장된 placement 정보는 파일에서 파싱하고 검증할 수 있지만, 기본 정책상 자동 Win32 제어나 외부 윈도우 재연결에는 사용하지 않는다.

tab preset은 설정 로드 시 app runtime preset 목록으로 복원한다. preset leaf는 region ID가 없는 template이므로 적용 시 `Workspace`의 `next_region_id` 흐름으로 새 region ID를 발급한다. 수동 설정 저장과 종료 직전 저장은 모두 `App::settings()`/`AppState::settings()`에서 같은 `WorkspaceSettings`를 만들기 때문에 preset 목록을 동일하게 보존한다. 종료 흐름에서는 이 설정 저장을 먼저 수행하고, 그 다음 runtime placement undock을 실행한다. entry 계층은 저장된 workspace session을 적용하지 않고 새 workspace로 시작한 사실, 설정 로드 실패 후 새 workspace로 시작한 사실, 종료 저장 실패와 undock 요약을 현재 UI 언어 기준 status로 표시하며 플랫폼별 구현은 같은 사용자 메시지를 사용한다.

현재 설정 schema는 `schema_version = 1`만 지원한다. 선택 필드인 `tab_presets`가 없으면 기존 파일 호환성을 위해 빈 목록으로 간주한다. `options`가 없으면 기본 UI 옵션으로 간주한다. 기존 설정 파일에 남아 있는 시작 복원 옵션 값은 호환성을 위해 무시한다. `schema_version`이 1이 아니면 설정 로드는 지원하지 않는 버전 오류로 거부한다.

## 11. 오류 처리 규칙

복구 가능한 오류는 `Result`로 반환한다.

도메인과 app 계층은 사용자에게 보여줄 메시지와 내부 원인 정보를 분리한다. Win32 API 실패 원인은 가능한 범위에서 `GetLastError` 또는 해당 API의 실패 정보를 보존한다.

예상 가능한 오류는 다음과 같다.

- invalid HWND
- 대상 window 접근 실패
- `ShowWindow` 또는 `SetWindowPos` 실패
- region 또는 tab ID 불일치
- 이미 배치된 window 재배치 충돌
- layout ratio 계산 실패
- 복원 대상 모니터 또는 좌표 변경

## 12. 계층 책임

### entry

- Windows message loop
- main window 생성
- 사용자 입력 이벤트 수신
- app use case 호출
- client/screen 좌표 변환과 UI geometry는 entry 하위 `ui` 모듈에 둔다.
- GDI drawing과 back buffer 자원 관리는 entry 하위 `gdi` 모듈에 둔다.

### app

- tab 추가/삭제/이름 변경
- tab 전환 흐름
- region split/delete 흐름
- placement/undock use case 조합
- 종료 시 전체 undock 조율

### domain

- tab, region, split, layout tree 모델
- TabId, RegionId, Rect, Placement, WindowSnapshot 같은 핵심 값 객체
- split ratio 계산 규칙
- region hit-test
- placement 상태 모델
- 배치 가능성 검증 규칙

### infra

- Win32 HWND 조회
- cursor position 조회
- mouse button state 조회
- `ShowWindow`, `SetWindowPos`, window rect/style 조회와 복원
- logging
- 설정 파일 저장/로드

### platform

Win32 통합이 커질 경우 `infra`에서 분리한다.

- DPI 처리
- monitor 좌표 변환
- Windows hook 또는 event subscription
- OS별 window capability 검사

Linux에서는 GTK4 entry가 UI 이벤트와 그리기를 담당하고, X11 전용 외부 창 제어는 `infra::LinuxWindowController`에 둔다. GTK4 entry는 `App<DefaultWindowController>`와 공통 `WindowController` 계약을 중심으로 사용하며, Win32 entry와 Linux GTK4 entry는 같은 app/domain 유스케이스를 호출한다. Linux entry에서만 필요한 hover tooltip 제목 조회와 tab preset 실행 후 window 재탐색은 X11 `_NET_WM_NAME`/`WM_NAME`, `_NET_CLIENT_LIST_STACKING`, `_NET_WM_PID`, `/proc` process tree scan을 사용한다.

## 13. 최소 유스케이스

### UC-01: 새 탭 추가

사용자가 새 탭 추가를 선택하면 j3GridDocker는 빈 root region을 가진 tab을 생성하고 tab 목록에 추가한다. 워크스페이스에 남은 tab이 없으면 새 tab 이름/ID와 root region ID는 0부터 다시 시작한다.

### UC-02: 탭 전환

사용자가 다른 tab을 선택하면 기존 active tab의 external window를 숨기고, 새 active tab의 external window를 표시한 뒤 region rect에 맞게 재배치한다.

### UC-03: 영역 분할

사용자가 region split을 선택하면 해당 region을 두 child region으로 나누고 splitter ratio를 초기값으로 저장한다.

### UC-04: splitter 조정

사용자가 splitter를 드래그하면 layout ratio가 갱신되고 active tab의 배치된 external window가 새 rect에 맞게 재배치된다.

### UC-05: 외부 윈도우 배치

사용자가 external window를 j3GridDocker region 안에 놓으면 j3GridDocker는 대상 HWND를 확인하고 해당 region에 placement를 생성한 뒤 window를 region 크기에 맞춘다.

이미 active tab에 배치된 external window를 다른 빈 region 안에 놓으면 새 snapshot을 만들지 않고 기존 placement의 region만 변경한 뒤 window를 새 region 크기에 맞춘다. 같은 region에 다시 놓은 경우에는 placement를 변경하지 않고 현재 region rect로 다시 보정한다. 대상 region에 다른 external window가 이미 배치되어 있으면 기존 `RegionAlreadyOccupied` 오류로 거부한다.

entry 계층은 active tab에 이미 배치된 external window가 선택되거나 OS 활성 창으로 관측되면 해당 region을 active region으로 동기화하고, status bar에 빈 영역 drop은 이동이고 j3GridDocker 바깥 drop은 detach라는 안내를 표시한다. 이동/재맞춤/배치 실패 status는 같은 generic domain 메시지만 노출하지 않고 사용자가 시도한 작업을 구분해 표시한다.

이미 active tab에 배치된 external window를 j3GridDocker window 바깥으로 드래그한 뒤 놓으면 placement를 제거하고 external window를 드롭 시점의 현재 screen rect에 유지한다. 이 detach는 일반 undock처럼 owner를 해제하고 style/ex-style 등 배치 중 변경된 window 속성은 가능한 범위에서 복원하되, 위치와 크기는 배치 전 snapshot rect가 아니라 드롭된 현재 rect를 사용한다. 이때 window는 정상 표시 상태로 남아야 한다. j3GridDocker window 내부지만 region 밖인 toolbar, tabbar, status bar 위에 놓은 경우에는 새 배치나 detach를 수행하지 않는다.

### UC-06: 배치 해제

사용자가 region의 해제 메뉴 또는 버튼을 선택하면 placement를 제거하고 external window를 배치 전 상태로 가능한 범위에서 복원한다. active tab의 docked external window를 클릭한 뒤 해제 버튼을 누른 경우에는 클릭된 HWND에 연결된 placement region을 대상으로 한다.

### UC-07: 탭 삭제

사용자가 tab을 삭제하면 해당 tab의 모든 placement를 undock하고 tab을 제거한다.

### UC-08: 프로그램 종료

사용자가 j3GridDocker를 종료하면 모든 tab의 모든 placement를 undock하고 숨겨진 external window도 표시 가능한 상태로 복원한다.

### UC-09: 탭 preset 저장, 편집, 삭제와 적용

사용자가 `Presets > Save Tab Preset...`을 선택하면 현재 tab의 splitter layout과 dock된 외부 프로그램 실행 정보를 이름 있는 tab preset으로 저장한다. 사용자는 `Presets > Edit Tab Preset`으로 이름, 실행 파일 경로, arguments를 편집할 수 있고, `Presets > Delete Tab Preset`으로 저장된 preset을 제거할 수 있다. 삭제된 preset을 다시 불러오려고 하면 찾을 수 없는 preset 오류가 반환되어야 한다. 사용자가 `Presets > Load Tab Preset`으로 preset을 선택하면 대상 tab의 기존 placement를 먼저 undock한 뒤 새 region ID를 발급한 layout과 프로그램 실행 복원을 적용한다. 설정을 저장하고 다시 로드하면 tab preset 목록은 app runtime으로 복원되며, 사용자는 새 탭에도 같은 preset을 다시 적용할 수 있다.

## 14. 구현 시 주의사항

- Win32 API 호출 실패를 무시하지 않는다.
- `ShowWindow` 호출 후 필요하면 `SetWindowPos`를 다시 호출해 최종 rect를 보정한다.
- j3GridDocker window가 minimized 상태가 되면 active tab window를 숨기고, restore/maximize 후 다시 표시 및 동기화한다.
- DPI awareness와 다중 모니터 좌표계를 초기 단계에서 명확히 정한다.
- polling 기반 드래그 감지는 과도한 CPU 사용을 피하도록 interval을 제한한다.
- 외부 window가 종료되거나 HWND가 재사용될 수 있으므로 매 조작 전 유효성을 확인한다.
- 배치 중인 external window가 사용자에 의해 직접 이동된 경우 active tab 동기화 정책에 따라 다시 region rect로 보정한다.
- 종료 중 복원 실패가 있어도 나머지 window 복원은 계속 진행한다.

## 15. 미결 정책

### 15.1 초기 구현에서 결정한 정책

- region에 이미 external window가 있을 때 새 external window를 drop한 경우 교체하지 않고 `RegionAlreadyOccupied` 오류로 거부한다.
- tab 삭제 후 다음 active tab은 삭제된 tab의 기존 목록 인덱스에 있던 다음 tab을 우선 선택하고, 다음 tab이 없으면 직전 tab을 선택한다. 마지막 tab 삭제 후에는 새 tab을 자동 생성하지 않고 active tab이 없는 빈 상태를 허용한다.
- tab 전환, active tab 배치 표시, inactive tab undock 전 표시 복원에는 기본적으로 `SW_SHOWNOACTIVATE`에 해당하는 no-activate 표시 정책을 사용한다.
- 초기 app 계층의 external window 재보정은 명시적 유스케이스 호출에 한정한다. tab 전환, region split/delete, placement 등록, active tab 동기화 호출에서 `SetWindowPos`에 해당하는 동기화가 수행되며, 별도 주기 polling이나 OS event subscription 기반 자동 재보정은 아직 포함하지 않는다.
- Win32 infra는 `cfg(windows)` 경계 안의 `Win32WindowController`가 담당한다. domain/app 계층은 `WindowHandle`, `Rect`, `WindowSnapshot` 같은 도메인 타입과 `WindowController` trait만 사용하며 `windows-sys` 타입을 직접 노출하지 않는다.
- active tab 위치 보정은 `SetWindowLongPtrW(GWLP_HWNDPARENT)`로 dock 중인 external window의 top-level owner를 j3GridDocker window로 설정한 뒤 `SetWindowPos`에 `SWP_NOACTIVATE`를 사용하고 `SWP_NOZORDER`를 사용하지 않아 현재 dock window를 non-topmost z-order 상단으로 올린다. 이어서 j3GridDocker owner window를 dock window 바로 뒤 z-order로 보정해 다른 top-level window가 j3GridDocker와 dock window 사이에 남지 않게 한다. undock/detach는 `SetWindowLongPtrW(GWLP_HWNDPARENT, 0)`으로 owner를 해제한다. undock rect 복원과 style 변경 후 frame 갱신은 기존 z-order를 흔들지 않도록 `SWP_NOZORDER`, `SWP_NOOWNERZORDER`, `SWP_NOACTIVATE`를 사용한다. `SetParent`를 호출하지 않고, `HWND_TOPMOST`를 사용하지 않는다.
- Win32 API 실패는 가능한 경우 `GetLastError` 값을 `WindowControlError`에 보존한다. `ShowWindow`의 반환값은 성공 여부가 아니라 이전 표시 상태이므로, 호출 전 HWND 유효성과 top-level 여부를 확인한다.
- main window의 초기 크기는 Win32와 Linux entry 모두 900x700으로 맞춘다.
- Windows entry 초기 구현은 custom-painted main window와 native top-level menu bar를 함께 사용한다. 표시되는 native title bar의 window title은 `j3GridDocker`로 설정한다. 메뉴바는 `Workspace`, `Layout`, `Presets`, `View`, `Options`, `Window`, `Help` 순서로 구성한다. 상단에는 탭바와 탭바 왼쪽의 고정 Show/Hide 버튼 및 New 버튼을 표시하고, 작업 영역 UI 표시 상태에서는 탭바 아래 선택 영역 대상 빠른 명령 툴바와 active tab의 region/splitter/status bar를 그린다. 영역 조작은 최상위 `Layout` 메뉴, 선택된 region 대상 빠른 버튼, 또는 region context menu로 처리한다. `About j3GridDocker`는 `Help` 메뉴에서 Win32 modal dialog로 연다. dialog 제목은 `About j3GridDocker`이고, 상단에는 Cargo package version 기반 `j3GridDocker {version}` 라벨을 표시한다. 본문은 `include_str!()`로 빌드에 포함한 `about.txt` 원문을 읽기 전용 멀티라인 스크롤 영역에 표시한다. 하단에는 GitHub 링크(`https://github.com/edgarp9`)와 OK 버튼을 표시한다. 링크를 누르면 기본 브라우저로 연다.
- 탭 overflow는 custom-painted 탭바 오른쪽의 dropdown 버튼으로 처리한다. entry 계층은 first visible tab index만 런타임 상태로 관리하고, active tab이 visible range 안에 들어오도록 탭 변경과 resize 시 보정한다. 저장 모델에는 overflow offset을 기록하지 않는다.
- 내부 UI paint는 client-local content bounds를 기준으로 계산하고, external window 배치와 drop 판정에만 screen rect를 사용한다. 순수 main window move는 client layout을 바꾸지 않으므로 내부 UI invalidate를 발생시키지 않고 active tab external window 위치만 screen rect 기준으로 동기화한다.
- custom paint는 background erase를 억제하고 memory DC back buffer에 그린 뒤 한 번에 복사해, title bar drag처럼 move message가 연속되는 구간에서도 region과 splitter가 중간 erase frame에 노출되지 않게 한다.
- Show/Hide 버튼으로 작업 영역 UI를 숨기면 탭바 아래 j3GridDocker 자체 UI, region 외곽선, splitter, 상태바, 메인 창 외곽 표시, j3GridDocker title bar, native top-level menu bar를 숨기고 docked external window만 보이게 한다. 이때 content area는 탭바 바로 아래부터 client 하단까지로 계산한다. 작업 영역 UI를 다시 표시하면 title bar와 native top-level menu bar를 복원하고 content area는 명령 툴바 아래부터 상태바 위까지로 계산한다. 두 경우 모두 active tab의 external window 위치와 크기를 현재 content area 기준으로 다시 동기화한다.
  j3GridDocker가 maximized 상태에서 작업 영역 UI를 숨길 때는 Windows maximized overlapped frame이 monitor 작업 영역 밖으로 확장되어 client origin을 음수로 밀 수 있다. 이 경우 숨김 상태 동안만 title bar와 resize frame을 제거한 borderless work-area bounds로 보정해 탭바의 화면 좌표와 external window content top을 같은 기준으로 맞추고, 작업 영역 UI를 다시 표시할 때 title bar와 resize frame을 복원한다. 이는 탭바 침범을 막기 위한 좌표계 보정이며 external window에 임의 여백을 추가하지 않는다.
- 작업 영역 UI 숨김 상태에서는 탭 전환, 새 탭 생성, Show/Hide 토글만 j3GridDocker UI 입력으로 처리하고, 탭바 드래그로 j3GridDocker window를 이동할 수 있다. 탭을 짧게 클릭하면 탭 전환으로 처리하고, 드래그로 판정되면 window 이동으로 처리한다. 이 상태에서는 tab reorder, region 선택, region context menu를 수행하지 않는다. 기본 옵션에서는 splitter drag와 새 external window 배치 drop 감지를 수행하지 않지만, `Dock While Workspace Controls Are Hidden` 옵션이 켜져 있으면 숨겨진 active tab layout bounds 기준으로 새 placement 등록, 기존 placement region 이동, Ctrl-held splitter overlay drag를 허용한다. 이미 active tab에 배치된 external window를 j3GridDocker window 바깥으로 놓는 detach 감지는 옵션과 관계없이 허용한다.
- 작업 영역 UI 표시 상태에서는 visible tab 본문을 threshold 이상 드래그하면 tab reorder를 시작한다. reorder 중 insertion 위치를 tab strip에 표시하고, release 시 `Workspace`/app의 `TabId` 기반 순서 변경 유스케이스로 커밋한다. overflow 상태에서는 visible tab midpoint 기준으로 insertion을 계산하며 edge hover 중 first visible tab index를 자동 조정한다.
- 작업 영역 UI 숨김 상태에서도 탭바가 보이면 tab 닫기 버튼은 표시 상태와 같은 의미로 동작한다. 닫기 버튼 영역 클릭은 window 이동 drag 시작이나 탭 본문 클릭과 충돌하지 않는다.
- tab context menu는 작업 영역 UI 표시/숨김과 관계없이 보이는 tab 위의 우클릭으로 열 수 있다. visible tab과 overflow dropdown이 아닌 탭바 빈 공간에서 우클릭하면 새 tab 생성, active tab 조작, main window 최소화/최대화 또는 복원/닫기를 위한 tabbar context menu를 연다. 메뉴 command id는 고정 command 범위 안에서 toolbar command와 충돌하지 않게 배정하고, overflow tab 선택 command는 별도 base range를 사용한다.
- `Close other tabs`는 best-effort 일괄 삭제 정책을 사용한다. 각 대상 tab은 기존 `delete_tab` app use case로 삭제하며, 실패한 tab은 해당 use case의 rollback 결과를 유지한 채 다음 대상 tab 삭제를 계속한다. status bar는 실패한 tab ID와 작업명을 포함해 사용자가 어떤 tab 작업이 실패했는지 알 수 있게 한다.
- main window의 resize/move, region split/delete, tab switch, tab 재선택, splitter drag 후에는 active tab placement를 현재 content area의 screen rect 기준으로 다시 동기화한다. 이 과정에서 stale placement가 제거될 수 있으므로 active tab occupied-region paint cache는 동기화 시도 후 무효화한다.
- splitter drag는 domain의 splitter hit-test 결과인 `SplitterPath`를 기준으로 ratio를 갱신한다. drag 중에는 app 유스케이스를 통해 active tab external window 위치/크기를 즉시 갱신한다.
- external window drop 감지는 Win32 timer 기반 polling으로 시작한다. 기본 interval은 125ms이며, 마우스 버튼을 누른 동안 cursor 아래 top-level HWND를 후보로 저장하고 후보 HWND의 window rect 변화를 함께 추적한다. 버튼을 놓은 위치가 active region이더라도 후보 window rect가 press 중 이동/크기 변경 threshold 이상 변하지 않았으면 placement 등록을 시도하지 않는다. cursor 아래 HWND가 active tab의 기존 placement이면 해당 placement region을 선택하되 drop 후보 추적은 계속 유지한다. 이후 이동 threshold를 넘긴 상태로 다른 빈 region에 놓으면 기존 placement를 새 region으로 이동한다. 이동 threshold를 넘긴 active tab의 기존 placement를 j3GridDocker window 바깥에 놓으면 해당 placement를 제거하고 현재 window rect를 유지한 채 detach한다. 작업 영역 UI 숨김 상태에서는 기본적으로 새 placement 등록과 region 이동을 하지 않고 기존 active placement의 바깥 detach만 처리한다. `Dock while hidden` 옵션이 켜져 있으면 숨김 상태에서도 active region drop을 배치/이동 후보로 처리한다. j3GridDocker window 내부지만 active region이 아닌 UI 영역에 놓은 경우에는 배치와 detach를 모두 수행하지 않는다.
- `Win32WindowController`는 j3GridDocker 자신의 HWND를 excluded owner window로 등록해 external placement 대상에서 제외한다.
- `icon.ico`는 Windows 빌드 시 `winresource` build script로 실행 파일의 기본 icon resource ID 1에 포함한다. 실행 시 main window icon은 `LoadImageW`로 해당 embedded resource를 우선 로드하고, 개발 중 리소스가 없을 때만 작업 디렉터리의 `icon.ico` 파일을 fallback으로 사용한다.
- Windows 릴리즈 실행 파일은 GUI 서브시스템으로 빌드해, 사용자가 직접 실행할 때 별도 콘솔 창을 띄우지 않는다. 디버그 빌드는 개발 중 `stderr` 진단을 바로 볼 수 있도록 콘솔 서브시스템을 유지한다.
- 설정 파일은 실행 파일과 같은 폴더에 `실행파일이름.toml` 형식으로 저장하며, 파일 I/O와 TOML DTO 변환은 infra에 둔다. domain/app은 `WorkspaceSettings` 값 객체만 다룬다. `tab_presets` 또는 `options`가 없는 schema v1 설정 파일은 각각 빈 preset 목록과 기본 옵션으로 로드한다. 기본 옵션은 숨김 상태 Dock 비활성화, UI 언어 영어다. 저장 DTO의 tab preset leaf는 새 `RegionId`를 기록하지 않으며, 로드된 preset 이름, 프로그램 실행 파일 경로, argument, ratio, layout depth 정책 위반은 DTO를 거친 뒤 domain 검증 오류로 거부한다.
- 프로그램 종료 시 현재 runtime placement가 남아 있는 상태에서 먼저 설정을 저장하고, 그 다음 모든 tab의 모든 placement를 수집한다. 종료 undock은 active/inactive 구분 없이 각 window를 먼저 표시 가능한 상태로 전환한 뒤 복원하며, 개별 undock 실패는 내부 로그에 남기고 나머지 복원을 계속한다.
- 다음 실행 시 저장된 HWND 자동 재연결은 지원하지 않는다. 저장된 placement의 복원 정책은 `SessionOnlyNoAutoRestore`이며, 시작 시 저장된 layout과 tab 메타데이터를 runtime 상태로 복원하지 않는다. 설정 파일의 tab preset 목록과 UI 옵션만 시작 시 runtime 상태로 반영한다.
- j3GridDocker window minimize 시 active tab의 external window는 `SW_HIDE`로 숨긴다. restore/maximize/resize 후에는 active tab placement를 현재 content area의 screen rect 기준으로 `SW_SHOWNOACTIVATE` 표시 후 `SetWindowPos`로 다시 동기화한다. minimized 중에는 splitter drag/drop 상태를 취소하고 위치 보정을 수행하지 않는다.

### 15.2 아직 미결인 정책

- 없음

## 16. Win32 infra 검증 절차

Win32 infra 변경 시 Windows target에서는 다음 자동 smoke test를 실행해 실제 top-level HWND 제어가 문서 정책을 따르는지 확인한다.

```text
cargo test infra::win32::tests::controller_smoke_places_and_restores_real_top_level_window -- --nocapture
cargo test infra::win32::tests::app_smoke_saves_snapshot_and_restores_real_top_level_window -- --nocapture
```

테스트는 offscreen test window를 생성하고 `WindowSnapshot` 저장, `ShowWindow`, `SetWindowPos`, `SW_HIDE`, style/ex-style 복원, parent 불변, TopMost 미사용을 확인한다. 로그에는 HWND, snapshot rect, 배치 rect, 복원 rect, style/ex-style, parent 값을 남겨 수동 재현 시 비교할 수 있게 한다.

## 17. 탭 UX 재현 테스트 계획

최근 탭 UX는 `entry`의 Win32 메시지 흐름, `entry::ui`의 탭바 geometry, `app`/`domain`의 상태 전이로 나누어 관측한다. 문제를 추측으로 수정하지 않기 위해 실패가 의심되는 입력 경계에서는 status bar와 단위 테스트 helper를 우선 사용하고, stderr에는 smoke 검증에 필요한 `tab-ux event=...` 수준의 이산 이벤트만 남긴다.

| 기능 | 현재 구현 상태 | 정상 경로 | 경계 조건 | 실패 조건 | 관측 지점 |
| --- | --- | --- | --- | --- | --- |
| 탭별 닫기 버튼 | visible tab rect 안에서 본문과 close button hit-test를 분리하고, close는 `delete_tab` app 흐름으로 들어간다. | active/inactive visible tab의 `X`를 클릭하면 placement undock 후 tab이 삭제되고 status bar에 삭제 tab, 현재 active tab, undock 집계가 표시된다. | overflow로 숨겨진 tab은 hit-test되지 않는다. 작업 영역 UI 숨김 상태에서도 보이는 close button은 window move drag보다 우선한다. 마지막 tab 삭제 후 active tab 없음이 허용된다. | layout bounds 계산 실패, undock restore 실패, active tab 삭제 후 자동 전환 실패. | `entry::ui::tab_hit_test_separates_body_and_close_button`, `app::delete_tab_*` 테스트, status bar의 `탭 삭제 ... Undock ...`, stderr `WindowControlError` 로그. |
| 탭 overflow 처리 | `first_visible_index`는 entry 런타임 상태로만 관리하고, dropdown에는 hidden tab만 command로 추가한다. | client width가 좁아지면 dropdown이 나타나고 숨겨진 tab 선택 시 `switch_tab` 후 active tab이 visible range 안으로 보정된다. | resize로 overflow가 사라지면 first visible index가 0으로 돌아간다. dropdown 영역은 tab hit-test에서 제외된다. visible tab rect는 viewport/dropdown과 겹치지 않아야 한다. | dropdown 생성 실패, 숨겨진 tab 없음, client-to-screen 변환 실패, overflow command id 소진. | `entry::ui` overflow layout/hit-test 테스트, `overflow_visible_tabs_stay_inside_viewport_and_before_dropdown` helper 테스트, stderr `tab-ux event=overflow-select ...`, 경계 status message. |
| 탭 우클릭 메뉴 | visible tab 위 `WM_RBUTTONUP`에서 tab context menu를 열고 `Rename tab`, `Close tab`, `Close other tabs`를 고정 command id로 처리한다. 탭바 빈 공간은 tabbar context menu로 처리한다. | tab 본문 또는 close button 영역에서 우클릭하면 같은 tab을 context target으로 잡고 선택 command를 실행한다. 탭바 빈 공간에서 우클릭하면 `New tab`, window minimize/maximize 또는 restore/close 항목이 표시되고 active tab이 있으면 active tab 조작 항목도 표시된다. | 작업 영역 UI 표시/숨김과 관계없이 보이는 tab과 탭바 빈 공간에서 동작한다. overflow dropdown은 빈 공간으로 취급하지 않는다. maximized 상태에서는 maximize 항목 대신 restore 항목을 표시한다. `Close other tabs` 대상이 없으면 no-op status를 표시한다. | popup menu 생성 실패, context target 누락, rename dialog 실패/취소/빈 이름, best-effort 삭제 중 일부 실패, window command dispatch 실패. | command id 충돌 테스트, `entry::ui::tab_strip_empty_hit_test_*`, context dispatch 테스트, window maximize/restore label 테스트, stderr `tab-ux event=context-action ...`, `Close other tabs ...` status와 실패 tab ID. |
| 탭 드래그 재정렬 | UI 표시 상태의 visible tab body에서 threshold 이상 이동하면 reorder drag로 전환하고, release 시 `TabId` 기준 `reorder_tab_before`를 호출한다. | drag insertion 위치가 바뀌고 release 시 tab order와 저장 설정 순서가 바뀌며 active tab ID는 유지된다. | 짧은 이동은 tab switch로 유지된다. close button/overflow는 reorder보다 우선한다. UI 숨김 상태의 threshold drag는 window move로 처리한다. overflow edge hover는 first visible index를 한 칸씩 이동한다. | mouse capture 상실, cancel mode, 대상 tab 누락, destination tab 누락, no-op insertion. | `domain`/`app` reorder 테스트, `entry::ui::tab_insertion_target_*`, `tab_reorder_auto_scroll_*`, status `탭 순서를 변경했습니다...` 또는 `변경하지 않았습니다...`, stderr `tab-ux event=reorder-* ...`. |
| 탭 전환 실패 피드백 | `app::switch_tab`이 대상 표시/배치와 이전 tab hide를 rollback하고, entry status가 대상/이전 active tab과 원인을 분리해 보여준다. | tab 클릭 또는 overflow 선택으로 target placement 표시, 위치 조정, 이전 active placement 숨김 순서가 수행된다. stale target placement는 제거 수를 성공 status에 포함한다. | 같은 active tab 재선택은 재표시로 구분한다. active tab 없음, target tab 없음, stale HWND, hidden workspace layout bounds를 각각 분리한다. | 대상 validate/show/set_position 실패, 이전 tab hide 실패, rollback 중 best-effort 실패, active tab 삭제 후 자동 전환 실패. | `app::switch_tab_*` failure/rollback 테스트, `entry` status text 테스트, status `대상 탭 창 ... 실패`, stderr error chain. |
| 탭 preset | `TabPreset`/`TabPresetNode`는 새 region ID를 저장하지 않고, app runtime 목록과 `WorkspaceSettings.tab_presets`로 왕복한다. leaf에는 선택적으로 외부 프로그램 실행 정보를 저장한다. | 현재 tab layout과 docked 프로그램 정보를 저장, 편집, 삭제하고, 다른 tab에 불러와 layout과 프로그램 실행 복원을 적용한다. 설정 저장/로드 후에도 같은 preset을 재사용할 수 있다. | 같은 이름 저장은 trim된 이름 기준으로 덮어쓴다. 삭제는 trim된 이름 기준으로 수행하며 active tab이나 placement를 변경하지 않는다. 대상 tab에 기존 placement가 있으면 먼저 undock한 뒤 적용한다. 적용 성공 시 대상 tab 이름은 preset 이름으로 바뀐다. | preset 이름 없음, preset 미존재, 대상 tab 없음, 프로그램 실행 파일 경로 없음, argument 오류, bounds가 너무 작아 active tab layout 검증 실패, 프로그램 실행/dock 실패. | `domain` tab preset 테스트, `app` tab preset 저장/적용/rollback 테스트, `infra::settings_file_store_round_trips_tab_presets`, `entry` tab preset status/helper 테스트, `scripts/ui_entry_smoke.ps1` 실행 가능 여부. |

수동 재현 시에는 다음 값을 함께 기록한다.

| 항목 | 기록 값 |
| --- | --- |
| tab 상태 | tab ID, tab 이름, active tab ID, tab 순서 |
| tabbar geometry | client width, `first_visible_index`, `visible_count`, dropdown 존재 여부 |
| 입력 | mouse down/up 위치, drag threshold 초과 여부, hit target(body/close/overflow) |
| app 결과 | `TabSwitchReport`, `TabDeletionReport`, undock attempted/restored/missing/failures |
| Win32 실패 | API 이름, HWND, `WindowOperation`, `GetLastError`, status bar 문구 |
