# 列挙型

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/syntax/type/11_enum.md%26commit_hash%3D14b0c449efc9e9da3e10a09c912a960ecfaf1c9d)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/syntax/type/11_enum.md&commit_hash=14b0c449efc9e9da3e10a09c912a960ecfaf1c9d)

列挙型(Enum type)はSetによって生成されます。
列挙型はそのままでも型指定で使えますが、クラス化したりパッチを定義することで更にメソッドを定義できます。
列挙型による部分型システムを列挙的部分型付けといいます。

```python
Bool = {True, False}
Status = {"ok", "error"}
```

`1..7`は`{1, 2, 3, 4, 5, 6, 7}`と書き換えられるので、要素が有限の場合は本質的に列挙型と区間型は等価です。

```python
Binary! = Class {0, 1}!.
    invert! ref! self =
        if! self == 0:
            do!:
                self.set! 1
            do!:
                self.set! 0

b = Binary!.new !0
b.invert!()
```

因みに、Ergの列挙型は他言語でよくある列挙型を包摂する概念です。

```rust
// Rust
enum Status { Ok, Error }
```

```python
# Erg
Status = {"Ok", "Error"}
```

Rustとの相違点は、構造的部分型(SST)を採用しているというところにあります。

```rust
// StatusとExtraStatusの間には何も関係がない
enum Status { Ok, Error }
enum ExtraStatus { Ok, Error, Unknown }

// メソッドを実装できる
impl Status {
    // ...
}
impl ExtraStatus {
    // ...
}
```

```python
# Status > ExtraStatusであり、Statusの要素はExtraStatusのメソッドを使える
Status = Trait {"Ok", "Error"}
    # ...
ExtraStatus = Trait {"Ok", "Error", "Unknown"}
    # ...
```

patchingによってメソッドの追加もできます。

明示的に包含関係を示したい場合、または既存のEnum型に選択肢を追加したい場合は`or`演算子を使います。

```python
ExtraStatus = Status or {"Unknown"}
```

要素の属するクラスがすべて同一である列挙型を等質(homogenous)な列挙型といいます。
デフォルトでは、等質な列挙型を要件型とするクラスは、要素が属しているクラスのサブクラスとして扱えます。
あえてそうしたくない場合は、ラッパークラスとするとよいでしょう。

```python
Abc = Class {"A", "B", "C"}
Abc.new("A").is_uppercase()

OpaqueAbc = Class {inner = {"A", "B", "C"}}.
    new inner: {"A", "B", "C"} = Self.new {inner;}
OpaqueAbc.new("A").is_uppercase() # TypeError
```
<p align='center'>
    <a href='./10_interval.md'>Previous</a> | <a href='./12_refinement.md'>Next</a>
</p>