# 宣言

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/syntax/03_declaration.md%26commit_hash%3D51de3c9d5a9074241f55c043b9951b384836b258)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/syntax/03_declaration.md&commit_hash=51de3c9d5a9074241f55c043b9951b384836b258)

宣言は、使用する変数の型を指定する構文です。
宣言はコード中のどこでも可能ですが、宣言しただけでその変数を参照することはできません。必ず初期化する必要があります。
代入後の宣言では、代入されたオブジェクトと型が適合するかをチェック可能です。

```python
i: Int
# i: Int = 2のように代入と同時に宣言できる
i = 2
i: Num
i: Nat
i: -2..2
i: {2}
```

代入後の宣言は`assert`による型チェックと似ていますが、コンパイル時にチェックされるという特徴があります。
実行時の`assert`による型チェックは「〇〇型かもしれない」で検査が可能ですが、コンパイル時の`:`による型チェックは厳密です。
「〇〇型である」ことが確定していなくては検査を通らず、エラーとなります。

```python
i = (-1..10).sample!()
assert i in Nat # これは通る可能性がある
i: Int # これは通る
i: Nat # これは通らない(-1はNatの要素ではないため)
```

関数は以下の2種類の方法で宣言が可能です。

```python
f: (x: Int, y: Int) -> Int
f: (Int, Int) -> Int
```

引数名を明示して宣言した場合、定義時に名前が違うと型エラーとなります。引数名の任意性を与えたい場合は2番目の方法で宣言すると良いでしょう。その場合、型検査で見られるのはメソッド名とその型のみです。キーワード指定による呼び出しはできなくなります。

```python
T = Trait {
    .f = (x: Int, y: Int): Int
}

C = Class(U, Impl := T)
C.f(a: Int, b: Int): Int = ... # TypeError: `.f` must be type of `(x: Int, y: Int) -> Int`, not `(a: Int, b: Int) -> Int`
```

<p align='center'>
    <a href='./02_name.md'>Previous</a> | <a href='./04_function.md'>Next</a>
</p>
