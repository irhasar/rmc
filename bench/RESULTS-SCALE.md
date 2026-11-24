# Compile-time scaling

Recorded 2026-11-14 15:21 UTC; CPU: Intel(R) Xeon(R) Processor @ 2.10GHz.
Regenerate with `python3 bench/scale.py --markdown bench/RESULTS-SCALE.md`.

Wall time of one build (best of three), in milliseconds. `front-end` is
`rustc --emit=metadata`, the type-checking floor every rustc pipeline pays.

| lines | rmc | cb | cranelift | llvm-O0 | llvm-O3 | front-end |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 973 | 13 | 205 | 85 | 76 | 158 | 65 |
| 9253 | 115 | 1029 | 406 | 428 | 1559 | 249 |
| 46053 | 1476 | 6472 | 2702 | 2233 | 16753 | 1182 |
