# OSS benchmark: full StrykerJS (A) vs Stryker incremental (B) vs rpr (C)

Runners: ubuntu24 20261018.96.1, ubuntu24 20261026.354.1, 4 CPUs; Node v24.22.1; rpr 0.3.0.

**Median wall speedup A/C over 31 PRs: 27.4×** (range 2.0× to 228.4×); CPU 54.1×. Verdict parity on the diff: 5193/5194 mutants with the same status (5194/5194 same detected/undetected), survivors 841/841. Failed or timed-out runs: 0 of 250; harness errors after a completed run (timing kept, no verdicts): 6.

Runner minutes: 1928.0 (A 1407.3, B 160.4, C 177.2, plan 0.3, scan 56.3, seed 126.4, setup 0.1).

| Repo | PRs | Median speedup wall / CPU | B/C wall | Parity (same/total) | Survivors (same/total, PRs) | Failed runs | Harness errors |
|---|---|---|---|---|---|---|---|
| boundaries-config | 3 | 21.9× / 35.7× | n/a | 85/85 | 0/0 (0 PRs) | 0 | 0 |
| boundaries-elements | 12 | 63.8× / 89.9× | n/a | 3106/3106 | 523/523 (3 PRs) | 0 | 0 |
| brazilian-utils | 6 | 18.9× / 29.2× | 4.2× | 1644/1645 | 0/0 (0 PRs) | 0 | 6 |
| trading-signals | 10 | 21.3× / 36.5× | 3.2× | 358/358 | 318/318 (8 PRs) | 0 | 0 |

| Repo | PR | A wall (n) | A CPU | A mutants / score | B wall (n) | C wall (n) | C mutants / score | Speedup wall / CPU | Parity A=C (survivors) | Failures |
|---|---|---|---|---|---|---|---|---|---|---|
| boundaries-config | #577 | 4.3 min (3) | 16.8 min | 1693 / 99.23 | n/a (0) | 16.5 s (3) | 43 / 100.0 | 15.8× / 27.5× | 43/43 (0/0) | - |
| boundaries-config | #578 | 6.0 min (3) | 23.5 min | 1698 / 99.23 | n/a (0) | 16.5 s (3) | 42 / 100.0 | 21.9× / 35.7× | 42/42 (0/0) | - |
| boundaries-config | #591 | 6.6 min (3) | 25.5 min | 1698 / 99.23 | n/a (0) | 8.5 s (3) | 0 / None | 46.3× / 125.8× | 0/0 (0/0) | - |
| boundaries-elements | #432 | 16.7 min (3) | 65.5 min | 5587 / 61.45 | n/a (0) | 2.5 min (3) | 1066 / 62.816901408450704 | 6.6× / 6.8× | 1066/1066 (396/396) | - |
| boundaries-elements | #453 | 38.6 min (3) | 152.3 min | 2497 / 94.11 | n/a (0) | 19.3 min (3) | 1901 / 93.37190952130457 | 2.0× / 2.0× | 1901/1901 (126/126) | - |
| boundaries-elements | #509 | 19.8 min (3) | 78.0 min | 2522 / 94.33 | n/a (0) | 24.5 s (3) | 10 / 100.0 | 48.5× / 68.3× | 10/10 (0/0) | - |
| boundaries-elements | #517 | 31.8 min (3) | 125.2 min | 2538 / 94.6 | n/a (0) | 21.5 s (3) | 16 / 100.0 | 88.6× / 119.8× | 16/16 (0/0) | - |
| boundaries-elements | #521 | 23.0 min (3) | 90.5 min | 2539 / 94.68 | n/a (0) | 28.5 s (3) | 13 / 100.0 | 48.4× / 69.6× | 13/13 (0/0) | - |
| boundaries-elements | #530 | 18.7 min (3) | 73.6 min | 2565 / 94.74 | n/a (0) | 56.0 s (3) | 62 / 100.0 | 20.0× / 23.7× | 62/62 (0/0) | - |
| boundaries-elements | #560 | 31.4 min (3) | 124.0 min | 2566 / 95.36 | n/a (0) | 41.5 s (3) | 24 / 100.0 | 45.4× / 56.8× | 24/24 (0/0) | - |
| boundaries-elements | #566 | 30.4 min (3) | 119.8 min | 2565 / 96.1 | n/a (0) | 23.0 s (3) | 14 / 92.3076923076923 | 79.2× / 110.3× | 14/14 (1/1) | - |
| boundaries-elements | #569 | 17.4 min (3) | 68.5 min | 2523 / 96.07 | n/a (0) | 10.5 s (3) | 0 / None | 99.6× / 253.9× | 0/0 (0/0) | - |
| boundaries-elements | #571 | 27.9 min (3) | 110.1 min | 2523 / 96.07 | n/a (0) | 12.5 s (3) | 0 / None | 134.1× / 349.7× | 0/0 (0/0) | - |
| boundaries-elements | #573 | 30.5 min (3) | 120.1 min | 2523 / 96.07 | n/a (0) | 8.0 s (3) | 0 / None | 228.4× / 605.8× | 0/0 (0/0) | - |
| boundaries-elements | #591 | 31.1 min (3) | 122.7 min | 2523 / 96.07 | n/a (0) | 10.0 s (3) | 0 / None | 186.7× / 464.1× | 0/0 (0/0) | - |
| brazilian-utils | #547 | 15.2 min (3) | 57.2 min | 4922 / 100.0 | 100.5 s (3) | 8.5 s (3) | 12 / 100.0 | 107.5× / 165.9× | 12/12 (0/0) | - |
| brazilian-utils | #556 | 16.4 min (3) | 61.7 min | 4953 / 100.0 | 3.2 min (3) | 33.0 s (3) | 91 / 100.0 | 29.7× / 48.7× | 91/91 (0/0) | - |
| brazilian-utils | #558 | 18.5 min (3) | 69.6 min | 5411 / 100.0 | 4.5 min (3) | 2.3 min (3) | 549 / 100.0 | 8.1× / 9.8× | 548/549 (0/0) | - |
| brazilian-utils | #592 | 15.8 min (3) | 59.4 min | 4947 / 100.0 | 52.0 s (3) | 7.5 s (3) | 4 / 100.0 | 126.0× / 220.9× | 4/4 (0/0) | - |
| brazilian-utils | #593 | 17.3 min (3) | 64.4 min | None / None | 11.6 min (3) | 7.5 min (3) | 1716 / 100.0 | 2.3× / 2.7× | n/a | B: 3; A: 3 |
| brazilian-utils | #615 | 20.4 min (3) | 76.3 min | 6199 / 100.0 | 17.4 min (3) | 6.9 min (3) | 989 / 100.0 | 3.0× / 3.4× | 989/989 (0/0) | - |
| trading-signals | #1318 | 2.4 min (3) | 8.4 min | 1816 / 33.59 | 11.5 s (3) | 9.0 s (3) | 2 / 0.0 | 16.1× / 26.8× | 2/2 (2/2) | - |
| trading-signals | #1365 | 2.5 min (3) | 8.5 min | 1816 / 33.59 | 38.5 s (3) | 5.5 s (3) | 2 / 50.0 | 26.8× / 54.1× | 2/2 (1/1) | - |
| trading-signals | #1373 | 2.2 min (3) | 7.6 min | 1826 / 35.32 | 43.5 s (3) | 12.0 s (3) | 36 / 83.33333333333333 | 11.0× / 14.8× | 36/36 (6/6) | - |
| trading-signals | #1374 | 2.5 min (3) | 8.6 min | 1813 / 34.03 | 17.5 s (3) | 5.0 s (3) | 2 / 100.0 | 29.7× / 60.7× | 2/2 (0/0) | - |
| trading-signals | #1375 | 2.6 min (3) | 9.1 min | 1820 / 34.73 | 13.5 s (3) | 8.5 s (3) | 8 / 75.0 | 18.6× / 29.7× | 8/8 (2/2) | - |
| trading-signals | #1397 | 2.9 min (3) | 10.9 min | 1959 / 2.09 | 42.5 s (3) | 29.5 s (3) | 279 / 0.0 | 5.8× / 6.6× | 279/279 (279/279) | - |
| trading-signals | #1398 | 2.4 min (3) | 9.1 min | 1965 / 2.09 | 18.0 s (3) | 6.0 s (3) | 9 / 0.0 | 24.0× / 43.2× | 9/9 (9/9) | - |
| trading-signals | #1415 | 2.7 min (3) | 10.5 min | 1971 / 2.08 | 16.0 s (3) | 6.0 s (3) | 6 / 0.0 | 27.4× / 56.1× | 6/6 (6/6) | - |
| trading-signals | #1417 | 2.8 min (3) | 10.7 min | 1981 / 2.07 | 39.0 s (3) | 10.5 s (3) | 13 / 0.0 | 16.0× / 23.0× | 13/13 (13/13) | - |
| trading-signals | #1424 | 2.8 min (3) | 10.9 min | 1982 / 2.12 | 26.5 s (3) | 6.0 s (3) | 1 / 100.0 | 28.4× / 70.0× | 1/1 (0/0) | - |
