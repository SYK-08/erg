# 變量和常量

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/syntax/02_name.md%26commit_hash%3D51de3c9d5a9074241f55c043b9951b384836b258)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/syntax/02_name.md&commit_hash=51de3c9d5a9074241f55c043b9951b384836b258)

## 變量
變量是一種代數； Erg 中的代數 - 如果沒有混淆，有時簡稱為變量 - 指的是命名對象并使它們可從代碼的其他地方引用的功能

變量定義如下
`n` 部分稱為變量名(或標識符)，`=` 是賦值運算符，`1` 部分是賦值

```python
n = 1
```

以這種方式定義的"n"此后可以用作表示整數對象"1"的變量。該系統稱為分配(或綁定)
我們剛剛說過`1`是一個對象。稍后我們將討論對象是什么，但現在我們假設它是可以賦值的，即在賦值運算符的右側(`=` 等)

如果要指定變量的"類型"，請執行以下操作。類型大致是一個對象所屬的集合，后面會解釋
這里我們指定`n`是自然數(`Nat`)類型

```python
n: Nat = 1
```

請注意，與其他語言不同，不允許多次分配

```python
# NG
l1 = l2 = [1, 2, 3] # 語法錯誤: 不允許多重賦值
# OK
l1 = [1, 2, 3]
l2 = l1.clone()
```

也不能重新分配給變量。稍后將描述可用于保存可變狀態的語法

```python,compile_fail
i = 1
i = i + 1 # 分配錯誤: 不能分配兩次
```

您可以在內部范圍內定義具有相同名稱的變量，但您只是覆蓋它，而不是破壞性地重寫它的值。如果您返回外部范圍，該值也會返回
請注意，這是與 Python "語句"范圍不同的行為
這種功能通常稱為陰影。但是，與其他語言中的陰影不同，您不能在同一范圍內進行陰影

```python
x = 0
# x = 1 # 賦值錯誤: 不能賦值兩次
if x.is_zero(), do:
    x = 1 # 與同名的外部 x 不同
    assert x == 1
assert x == 0
```

乍一看，以下內容似乎可行，但仍然不可能。這是一個設計決定，而不是技術限制

```python
x = 0
if x.is_zero(), do:
    x = x + 1 # 名稱錯誤: 無法定義變量引用同名變量
    assert x == 1
assert x == 0
```

## 常量

常數也是一種代數。如果標識符以大寫字母開頭，則將其視為常量。它們被稱為常量，因為一旦定義，它們就不會改變
`N` 部分稱為常量名(或標識符)。否則，它與變量相同

```python
N = 0
if True, do:
    N = 1 # 賦值錯誤: 常量不能被遮蔽
    pass()
```

常量在定義的范圍之外是不可變的。他們不能被遮蔽。由于這個屬性，常量可以用于模式匹配。模式匹配在后面解釋

例如，常量用于數學常量、有關外部資源的信息和其他不可變值

除了 [types](./type/01_type_system.md) 之外的對象標識符使用全大寫(所有字母大寫的樣式)是常見的做法

```python
PI = 3.141592653589793
URL = "https://example.com"
CHOICES = ["a", "b", "c"]
```

```python
PI = 3.141592653589793
match! x:
    PI => print! "π"
    other => print! "other"
```

當 `x` 為 `3.141592653589793` 時，上面的代碼會打印 `π`。如果 `x` 更改為任何其他數字，它會打印 `other`

有些對象不能綁定為常量。例如，可變對象。可變對象是其狀態可以改變的對象，后面會詳細介紹
這是因為只有常量表達式才能分配給常量的規則。常量表達式也將在后面討論

```python
X = 1 # OK
X = !1 # 類型錯誤: 無法定義 Int！ 對象作為常量
```

## 刪除變量

您可以使用 `Del` 函數刪除變量。依賴于變量的所有其他變量(即直接引用變量值的變量)也將被刪除

```python
x = 1
y = 2
Z = 3
f a = x + a

assert f(2) == 3
Del x
Del y, Z

f(2) # 名稱錯誤: f 未定義(在第 6 行中刪除)
```

注意 `Del` 只能刪除用戶自定義模塊中定義的變量。無法刪除諸如"True"之類的內置常量

```python
Del True # 類型錯誤: 無法刪除內置常量
Del print! # TypeError: 無法刪除內置變量
```

## 附錄: 賦值和等價

請注意，當 `x = a` 時，`x == a` 不一定為真。一個例子是`Float.NaN`。這是 IEEE 754 定義的浮點數的正式規范

```python
x = Float.NaN
assert x ! = NaN
assert x ! = x
```

還有其他對象首先沒有定義等價關系

```python,compile_fail
f = x -> x**2 + 2x + 1
g = x -> (x + 1)**2
f == g # 類型錯誤: 無法比較函數對象

C = Class {i: Int}
D = Class {i: Int}
C == D # 類型錯誤: 無法比較類對象
```

嚴格來說，`=` 不會將右側的值直接分配給左側的標識符
在函數和類對象的情況下，執行"修改"，例如將變量名稱信息賦予對象。但是，結構類型并非如此

```python
f x = x
print! f # <函數 f>
g x = x + 1
print! g # <函數 g>

C = Class {i: Int}
print! C # <類 C>
```

<p align='center'>
    <a href='./01_literal.md'>上一頁</a> | <a href='./03_declaration.md'>下一頁</a>
</p>
