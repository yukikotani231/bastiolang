# Bastio

メモリ安全性をコンパイル時に**証明する** OS 記述言語です。fz/fzl の後継として書き直しています。この言語で書く OS は BastiOS(`bastios`)の予定です。

- **unsafe がない**:生のアドレスからメモリを読む手段が、言語に存在しません。
- **線形な所有権**:バッファは必ずちょうど 1 回消費されます。解放後の使用、二重解放、**リーク**はコンパイルエラーになります。
- **範囲と算術の証明**:配列の添字、オーバーフロー、アンダーフロー、ゼロ除算を、Z3(SMT ソルバー)がコンパイル時に証明します。
- **ゼロオーバーヘッド**:証明はすべてコンパイル時に消えます。生成コードには境界チェックも実行時検査もありません。
- **関数の正しさは任意(opt-in)**:`requires`/`ensures`/`invariant` を書いた関数だけ、仕様どおりであることを証明します。

設計の背景と Rust、Verus、ATS との比較は、設計メモ(Claude Doc)にあります。

## 例

```
fn fill(mut b: Buf, v: u64) -> Buf
  ensures len(result) == len(b)
  ensures forall j: u64 :: j < len(result) ==> get(result, j) == v
{
  let mut i = 0;
  while i < len(b)
    invariant i <= len(b)
    invariant forall j: u64 :: j < i ==> get(b, j) == v
  {
    b = set(b, i, v);   // i < len(b) はループ条件から自動で証明される
    i = i + 1;          // オーバーフローしないことも自動で証明される
  }
  return b;
}
```

よくある off-by-one(`while i <= len(a)`)を書くと、反例付きで拒否されます。

```
tests/fail/oob_loop.bastio:9:9: error[E-VERIFY-BOUNDS]: could not prove that the index `i` is within `a` (`i < len(a)`)
  |     a = set(a, i, 1);
  |         ^
  = counterexample: len(a) = 4, i = 4
```

## ビルドとテスト

必要なもの:Rust(stage0 のビルド用)、`z3`、C コンパイラ(`cc`)。

```sh
cd stage0 && cargo build --release
tests/run.sh                       # 通るべき例、拒否されるべき例、ゼロオーバーヘッド、サニタイザー照合
stage0/target/release/bastioc0 -o hello examples/hello.bastio && ./hello
```

`bastioc0` のオプションは次のとおりです。
- `--check`:検証だけ行う
- `--emit-c`:C を出力する
- `--dump-smt`:ソルバーに送った問い合わせを表示する

## 構成

| パス | 中身 |
| --- | --- |
| `stage0/` | 使い捨てのブートストラップコンパイラ(Rust)。selfhost が成立したら凍結する |
| `tests/pass/` | 検証を通り、期待した終了コードで動くべきプログラム |
| `tests/fail/` | 決まったエラーコードで拒否されるべきプログラム |
| `docs/language.md` | v0 の言語仕様 |
| `ROADMAP.md` | selfhost とカーネルまでの道筋 |

## 現在地(v0)

型は `u64`、`bool`、`Buf`(u64 の所有バッファ)、`&Buf`(読み取り専用の借用)だけです。バックエンドは C 出力です。v0 で保証することと保証しないことの一覧は、[docs/language.md](docs/language.md) の「健全性の範囲」にあります。
