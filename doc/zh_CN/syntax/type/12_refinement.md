# 细化类型

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/syntax/type/12_refinement.md%26commit_hash%3Dc248056b7e0273027b3c86fb912430bbde711941)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/syntax/type/12_refinement.md&commit_hash=c248056b7e0273027b3c86fb912430bbde711941)

细化类型是受谓词表达式约束的类型。枚举类型和区间类型是细化类型的语法糖

细化类型的标准形式是`{Elem: Type | (Pred)*}`。这意味着该类型是其元素为满足 `Pred` 的 `Elem` 的类型
可用于筛选类型的类型仅为 [数值类型](./08_value.md)

```python
Nat = 0.. _
Odd = {N: Int | N % 2 == 1}
Char = StrWithLen 1
# StrWithLen 1 == {_: StrWithLen N | N == 1}
[Int; 3] == {_: Array Int, N | N == 3}
Array3OrMore == {A: Array _, N | N >= 3}
```

当有多个 pred 时，可以用 `;` 或 `and` 或 `or` 分隔。`;` 和 `and` 的意思是一样的

`Odd` 的元素是 `1, 3, 5, 7, 9, ...`
它被称为细化类型，因为它的元素是现有类型的一部分，就好像它是细化一样

`Pred` 被称为(左侧)谓词表达式。和赋值表达式一样，它不返回有意义的值，左侧只能放置一个模式
也就是说，诸如`X**2 - 5X + 6 == 0`之类的表达式不能用作细化类型的谓词表达式。在这方面，它不同于右侧的谓词表达式

```python
{X: Int | X**2 - 5X + 6 == 0} # 语法错误: 谓词形式无效。只有名字可以在左边
```

如果你知道如何解二次方程，你会期望上面的细化形式等价于`{2, 3}`
但是，Erg 编译器对代数的了解很少，因此无法解决右边的谓词

## 智能投射

很高兴您定义了 `Odd`，但事实上，它看起来不能在文字之外使用太多。要将普通 `Int` 对象中的奇数提升为 `Odd`，即将 `Int` 向下转换为 `Odd`，您需要传递 `Odd` 的构造函数
对于细化类型，普通构造函数 `.new` 可能会出现恐慌，并且有一个名为 `.try_new` 的辅助构造函数返回一个 `Result` 类型

```python
i = Odd.new (0..10).sample!()
i: Odd # or Panic
```

它也可以用作 `match` 中的类型说明

```python
# i: 0..10
i = (0..10).sample!
match i:
    o: Odd ->
        log "i: Odd"
    n: Nat -> # 0..10 < Nat
        log "i: Nat"
```

但是，Erg 目前无法做出诸如"偶数"之类的子决策，因为它不是"奇数"等

## 枚举、区间和筛选类型

前面介绍的枚举/区间类型是细化类型的语法糖
`{a, b, ...}` 是 `{I: Typeof(a) | I == a 或 I == b 或 ... }`，并且 `a..b` 被去糖化为 `{I: Typeof(a) | 我 >= a 和我 <= b}`

```python
{1, 2} == {I: Int | I == 1 or I == 2}
1..10 == {I: Int | I >= 1 and I <= 10}
1... <10 == {I: Int | I >= 1 and I < 10}
```

## 细化模式

正如 `_: {X}` 可以重写为 `X`(常量模式)，`_: {X: T | Pred}` 可以重写为`X: T | Pred`

```python
# 方法 `.m` 是为长度为 3 或更大的数组定义的
Array(T, N | N >= 3)
    .m(&self) = ...
```
<p align='center'>
    <a href='./11_enum.md'>上一页</a> | <a href='./13_algebraic.md'>下一页</a>
</p>