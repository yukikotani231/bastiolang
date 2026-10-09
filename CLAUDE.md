# Bastio(言語)

メモリ安全性をコンパイル時に証明する OS 記述言語。fz/fzl の後継。

- 設計方針と道筋: [ROADMAP.md](ROADMAP.md)(不変ルールもここ)
- 言語仕様: [docs/language.md](docs/language.md)

## ビルドとテスト

```sh
cd stage0 && cargo build --release     # 警告はエラー扱い(CI 準拠)
tests/run.sh                           # 要: z3, cc
```

`tests/run.sh` の件数は手で書き写さないこと(fz で数字が腐り続けた教訓)。実物を数える:
`ls tests/pass tests/fail | grep -c Bastio`

## 機能追加のルール

- 新しい保証を足したら、それを**破る**プログラムを `tests/fail/` に足し、正しいエラーコードで落ちることを確かめる
- 生成コードに実行時検査を足して安全性を得てはいけない(ゼロオーバーヘッドの原則)。証明で得る
- 信頼する部分(Z3、C コンパイラ、ランタイムのプレリュード)を増やすときは ROADMAP の不変ルール 3 に従う

## コミット規約

`<area>: <summary>`。area は `stage0` / `lang` / `test` / `docs` / `ci` など。本文には何を証明できるようになったか、何がまだ証明されないかを正直に書く。
