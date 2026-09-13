# 로컬 테스트 매트릭스 (사용자 작성)

각 항목을 실제로 돌려보고 결과를 적는다. 완료 후 이 파일 + `*-report.txt` + `poc3-report.json` 을 전달하면
항목별로 해석하고, 실패 항목은 원인 분석 → 수정안을 제시한다.

## 0. 환경

| 항목 | 값 |
|---|---|
| OS / 버전 | |
| GPU / 드라이버 | |
| 데스크톱 환경(x11/wayland) | |
| 입력기(fcitx5/ibus/기본) 및 버전 | |
| Rust 버전(`rustc -V`) | |

## 1. PoC1 — Slint 외부 텍스처

| 확인 | shared | copy |
|---|---|---|
| 애니메이션 패턴 정상 표시 | | |
| fps | | |
| 프레임당 시간(ms) | | |
| GPU 사용률 상승 확인 | | |
| 리사이즈/최대화 후 정상 | | |
| HiDPI(스케일 125/150/200%) 정상 | | |
| 다중 모니터 이동 시 정상 | | |
| stderr 에 wgpu validation error 없음 | | |

## 2. PoC2 — IME (docs/poc2-ime-matrix.md 표를 채운다)

| 확인 | Windows | Linux Wayland | Linux X11 | macOS |
|---|---|---|---|---|
| preedit 가 창 제목에 실시간 표시 | | | | |
| Commit 후 preedit 초기화 | | | | |
| 후보창이 캐럿을 따라옴 | | | | |
| 한자 변환 | | | | |
| 조합 중 포커스 상실 → 복귀 | | | | |
| Slint 위젯에서 한글 입력/확정 | | | | |
| 조합 중간값이 `text` 로 들어옴 | | | | |

## 3. PoC3 — CubeCL 디바이스 공유

| 확인 | 결과 |
|---|---|
| device=Existing 출력 | |
| scale PASS / blur PASS | |
| 첫 호출(컴파일) ms | |
| kernel ms/frame | |
| read_one ms/frame | |
| WGSL compute ms/frame | |
| upload ms/frame | |
| `--device cpu` 도 동작(GPU 없이) | |

## 4. 종합 판단에 필요한 사용자 의견

- [ ] Slint 위젯의 기본 룩앤필로 "미려한 디자인"이 가능해 보이는가(위젯 커스터마이즈 난이도 체감)
- [ ] IME 품질이 실사용 가능한 수준인가(오타/누락/후보창 위치)
- [ ] CubeCL 을 프레임 경로에 두는 것이 합리적인가, 오프라인 작업에 두는 것이 합리적인가
