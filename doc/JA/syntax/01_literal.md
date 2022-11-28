# リテラル

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/syntax/01_literal.md%26commit_hash%3D00350f64a40b12f763a605bc16748d09379ab182)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/syntax/01_literal.md&commit_hash=00350f64a40b12f763a605bc16748d09379ab182)

## 基本的なリテラル

### 整数リテラル(Int Literal)

```python
0, -0, 1, -1, 2, -2, 3, -3, ...
```

整数(Int)リテラルはInt型のオブジェクトです。

> __Note__: `Int`型の部分型として`Nat`型が存在します。
> 0以上の数値は`Nat`型とも解釈できます。

### 有理数リテラル(Ratio Literal)

```python
0.00, -0.0, 0.1, 400.104, ...
```

有理数を表すリテラルです。専ら小数として表現されますが、内部的には分数として扱われます。
`Ratio`リテラルで整数部分または小数部分が`0`のときは、その`0`を省略できます。

```python
assert 1.0 == 1.
assert 0.5 == .5
```

> __Note__: この`assert`という関数は、`1.0`と`1.`が等しいことを示すために使用しました。
> 以降のドキュメントでは、結果が等しいことを示すために`assert`を使用する場合があります。

### 文字列リテラル(Str Literal)

Unicodeで表現可能な文字列は、すべて使用できます。
Pythonとは違い、`'`ではクオーテーション(囲み)できません。文字列の中で`"`を使いたいときは`\"`としてください。

```python
"", "a", "abc", "111", "1# 3f2-3*8$", "こんにちは", "السَّلَامُ عَلَيْكُمْ", ...
```

`{}`によって文字列の中に式を埋めこめます。これを文字列補間(string interpolation)といいます。
`{`, `}`自体を出力したい場合は`\{`, `\}`とします。

```python
assert "1 + 1 is 2" == "{1} + {1} is {1+1}"
s = "1+1"
assert "\{1+1}\" == "\{{s}\}"
```

### 指数リテラル(Exponential Literal)

これは学術計算でよく使用される指数表記を表すリテラルです。`Ratio`型のインスタンスになります。
非常に大きな/小さな数を表すときに使用します。Pythonと表記法は同じです。

```python
1e-34, 0.4e-10, 2.455+e5, 245e5, 25E5, ...
```

```python
assert 1e-10 == 0.0000000001
```

## リテラルを組み合わせて生成するもの(複合リテラル)

これらのリテラルは、それぞれ単独で解説されているドキュメントがあるので、詳しくはそちらを参照してください。

### [配列リテラル(Array Literal)](./10_array.md)

```python
[], [1], [1, 2, 3], ["1", "2",], ...
```

### [組リテラル(Tuple Literal)](./11_tuple.md)

```python
(), (1, 2, 3), (1, "hello", True), ...
```

### [辞書リテラル(Dict Literal)](./12_dict.md)

```python
{:}, {"one": 1}, {"one": 1, "two": 2}, {"1": 1, "2": 2}, {1: "1", 2: True, "three": [1]}, ...
```

### [レコードリテラル(Record Literal)](./13_record.md)

```python
{=}, {one = 1}, {one = 1; two = 2}, {.name = "John"; .age = 12}, {.name = Str; .age = Nat}, ...
```

### [集合リテラル(Set Literal)](./14_set.md)

```python
{}, {1}, {1, 2, 3}, {"1", "2", "1"}, ...
```

`Array`リテラルとの違いとして、`Set`では重複する要素が取り除かれます。

```python
assert {1, 2, 1} == {1, 2}
```

### リテラルのように見えるがそうではないもの

## 真偽値オブジェクト(Boolean Object)

```python
True, False
```

真偽値オブジェクトはBool型の単なるシングルトン(ダブルトン?)です。
Pythonからの伝統により、`Bool`型は`Int`型ないし`Nat`型のサブタイプとなります。
すなわち、`True`は`1`、`False`は`0`と解釈できます。

```python
assert True * 2 == 2
```

### Noneオブジェクト

```python
None
```

`NoneType`型のシングルトンです。

## 範囲オブジェクト(Range Object)

```python
assert 0..10 in 5
assert 0..<10 notin 10
assert 0..9 == 0..<10
assert (0..5).to_set() == {1, 2, 3, 4, 5}
assert "a" in "a".."z"
```

Pythonの`range`とは異なり、IntだけでなくStrオブジェクトなども範囲として扱うことができます。

## 浮動小数点数オブジェクト(Float Object)

```python
assert 0.0f64 == 0
assert 0.0f32 == 0.0f64
```

`Ratio`オブジェクトに`Float 64`の単位オブジェクトである`f64`を乗算したものです。
誤差が生じる可能性がありますが、`Ratio`よりも高速に計算できます。

## 複素数オブジェクト(Complex Object)

```python
1+2Im, 0.4-1.2Im, 0Im, Im
```

`Complex`オブジェクトは、単に虚数単位オブジェクトである`Im`との演算の組み合わせで表します。

## *-less multiplication

Ergでは、解釈に紛れがない限り乗算を表す`*`を省略できます。
ただし、演算子の結合強度は`*`よりも強く設定されています。

```python
# `assert (1*m) / (1*s) == 1*(m/s)`と同じ
assert 1m / 1s == 1 (m/s)
```

<p align='center'>
    <a href='./00_basic.md'>Previous</a> | <a href='./02_name.md'>Next</a>
</p>
