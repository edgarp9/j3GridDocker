# License Compliance Notes

이 문서는 2026-06-21 기준 `Cargo.lock`, 전체 타깃 Cargo 의존성, 포함 리소스로 확인한 라이선스 준수 메모다.

## Scope

- Rust crates: `cargo metadata --format-version 1 --locked`와 `cargo tree --locked --target all --edges normal,build` 기준.
- 보조 스크립트: `build_release.py`는 Python 표준 라이브러리만 사용한다.
- 포함 리소스: `icon.svg`, `icon.ico`.
- Linux native libraries: `*-sys` 크레이트의 `metadata.system-deps` 기준으로 식별했다.

## Required Distribution Files

- 프로젝트 GPL-3.0-or-later 본문은 저장소 루트의 `LICENSE`에 두고, `Cargo.toml`은 `license = "GPL-3.0-or-later"`로 표기한다.
- Rust crate와 포함 리소스 고지는 저장소 루트의 `THIRD_PARTY_NOTICES.txt`에 둔다.
- About 창은 배포물의 `about.txt`와 같은 내용을 빌드에 포함해 프로그램 이름, 버전, 프로젝트 저작권 확인 필요 표시, GPL-3.0-or-later 배포 문구, `LICENSE` 전문 경로, `THIRD_PARTY_NOTICES.txt` 고지 경로, 소스코드 제공 안내를 표시한다.
- About 창의 `Licenses` 버튼은 프로젝트 GPL-3.0-or-later 고지와 빌드 시점의 `THIRD_PARTY_NOTICES.txt`를 바이너리에 포함해 표시한다.
- `build_release.py --no-open` 또는 `build_release.py`로 release build를 만들면 `LICENSE`, `THIRD_PARTY_NOTICES.txt`, `about.txt`를 `target/release`로 복사한다.
- 같은 릴리스 스크립트는 GPL-3.0-or-later 바이너리 배포에 필요한 프로젝트 대응 소스 제공을 놓치지 않도록 `target/release/j3grid-docker-<version>-source.zip`도 생성한다.
- 같은 릴리스 스크립트는 Windows 바이너리 ZIP도 만들고, 그 안에 실행 파일, `LICENSE`, `THIRD_PARTY_NOTICES.txt`, `about.txt`, 대응 소스 ZIP을 포함한다.
- 의존성 변경 뒤에는 `Cargo.lock`을 갱신한 다음 `THIRD_PARTY_NOTICES.txt`를 다시 생성해야 한다.

## GPL-3.0-or-later Release Handling

바이너리 배포물에는 최소한 다음 항목을 함께 둔다.

- `LICENSE`
- `THIRD_PARTY_NOTICES.txt`
- `about.txt`
- `j3grid-docker-<version>-source.zip`

소스 아카이브는 저장소의 빌드 가능한 프로젝트 소스, `Cargo.toml`, `Cargo.lock`, 빌드 스크립트, 문서와 라이선스 파일을 포함하고, VCS/빌드 산출물/편집기 설정/리포트/임시 파일은 제외한다.

이 아카이브는 프로젝트 자체의 대응 소스 제공을 자동화하기 위한 것이다. 완전히 독립적인 오프라인 소스 배포가 필요하거나 Cargo registry 크레이트 소스까지 같은 배포물에 포함해야 하는 정책이라면, `cargo vendor` 또는 별도 vendor archive 정책을 추가로 확정해야 한다.

## Rust Crate Result

Rust crate 라이선스 표현은 현재 locked Cargo metadata resolve graph 83개 패키지 기준으로 모두 GPL-3.0-or-later 프로젝트와 함께 고지 가능한 계열로 확인됐다. 각 항목의 이름, 버전, 라이선스 표현, copyright/notice 문구 또는 확인 필요 표시, upstream URL, 수정 여부, 배포 포함 여부는 `THIRD_PARTY_NOTICES.txt`의 `Third-Party Component Summary`에 둔다.

- MIT
- Apache-2.0
- Apache-2.0 WITH LLVM-exception
- Unlicense
- Unicode-3.0

`OR`가 포함된 dual license 표현은 임의로 하나를 선택하지 않고, `THIRD_PARTY_NOTICES.txt`에 upstream 표현과 포함된 라이선스 파일을 그대로 보존한다.

## Bundled Resource Result

- `icon.svg`, `icon.ico`: Google Fonts Icons / Material Symbols 계열에서 유래한 아이콘 리소스로 확인했다. 프로젝트 소유자 제공 정보는 SIL Open Font License 1.1이지만, Google Material Symbols 공식 가이드와 upstream repository는 Apache-2.0으로 고지하므로 최종 라이선스는 확인 필요로 분류했다.
- 해당 리소스 고지는 `THIRD_PARTY_NOTICES.txt`에 추가했다. Apache-2.0과 OFL-1.1 전문을 하단 공통 라이선스 전문 섹션에 포함했다.

## Native Library Result

Linux 빌드에서 Cargo가 찾는 native library는 다음 계열이다.

- Cairo: upstream은 LGPL-2.1 또는 MPL-1.1 선택 가능으로 고지한다.
- GTK4/GDK/GSK, GLib/GObject/Gio, Pango/PangoCairo: 공식 문서는 LGPL-2.1-or-later로 고지한다.
- GdkPixbuf: GNOME GitLab project page는 LGPL-2.1-or-later를 고지하고 upstream `COPYING`은 LGPL-2.1 본문을 포함하지만, 현재 generated docs 페이지의 license 표기는 `GPL-2.1-or-later`로 보인다. 번들 배포 전 실제 사용하는 소스 패키지의 라이선스 파일로 재확인해야 한다.
- Graphene: upstream은 MIT/X11로 고지한다.

native library를 OS 패키지로만 요구하고 앱 배포물에 포함하지 않는 경우에는 배포 문서에 필요한 OS 패키지 이름과 버전을 기록한다. shared library를 앱과 함께 번들링하는 경우에는 해당 바이너리와 함께 upstream 라이선스 파일, 저작권 고지, LGPL/MPL 소스 또는 relinking 의무 충족 자료를 포함해야 한다.

## Legal Review Items

정책 판단이 필요한 항목은 임의로 처리하지 않았다.

- 이 프로젝트 자체의 배포 라이선스는 GPL-3.0-or-later로 확인됐다. 현재 `Cargo.toml`은 `license = "GPL-3.0-or-later"`로 SPDX 표기를 둔다.
- 프로젝트 자체 copyright holder 문구가 `Cargo.toml`, `LICENSE`, 소스 헤더, 문서에서 확인되지 않았다. About 창과 라이선스 고지에는 임의 저작권자를 넣지 않고 `확인 필요`로 표시했다.
- LGPL native library를 번들링할지, OS 패키지 의존으로 둘지 배포 정책 결정이 필요하다.
- LGPL native library를 정적으로 링크하거나 수정본을 포함하는 배포는 법무 검토가 필요하다.
- Cairo의 LGPL-2.1/MPL-1.1 중 어떤 조건으로 배포 고지를 구성할지 정책 결정이 필요하다.
- GdkPixbuf는 실제 번들링 대상 버전의 source package에서 최종 라이선스 표기를 확인해야 한다.
- Cargo registry 크레이트 소스를 릴리스 ZIP에 vendoring할지, `Cargo.lock`과 crates.io의 동일 버전 source package 접근으로 충분하다고 볼지 법무/배포 정책 판단이 필요하다.
- 아이콘 리소스는 Google Fonts Icons 페이지 기준 SIL Open Font License 1.1이라는 프로젝트 소유자 제공 정보와 Google Material Symbols 공식 가이드의 Apache-2.0 표기가 충돌한다. `icon.svg`와 `icon.ico`의 정확한 upstream asset 이름, download-time license 표시, 변환 이력을 확인해야 한다.
