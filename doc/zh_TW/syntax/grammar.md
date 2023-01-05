# Erg 的語法(版本 0.1.0, 臨時)

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/syntax/grammar.md%26commit_hash%3Df09ef75b8b7bb86f892f224db4392c7c340b1147)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/syntax/grammar.md&commit_hash=f09ef75b8b7bb86f892f224db4392c7c340b1147)

```
special_op ::= '=' | '->' | '=>' | '.' | ',' | ':' | '::' | '|>' | '&'
separator ::= ';' | '\n'
escape ::= '\'
comment_marker ::= '#'
reserved_symbol ::= special_op | separator | comment_marker
number ::= [0-9]
first_last_dight ::= number
dight ::= number | '_'
bin_dight ::= [0-1]
oct_dight ::= [0-8]
hex_dight ::= [0-9]
        | [a-f]
        | [A-F]
int ::= first_last_dight
        | first_last_dight dight* first_last_dight
        | '0' ('b' | 'B') binary_dight+
        | '0' ('o' | 'O') octa_dight+
        | '0' ('x' | 'X') hex_dight+
ratio ::= '.' dight* first_last_dight
        | first_last_dight dight* '.' dight* first_last_dight
bool ::= 'True' | 'False'
none ::= 'None'
ellipsis ::= 'Ellipsis'
not_implemented ::= 'NotImplemented'
parenthesis ::= '(' | ')'
bracket ::= '{' | '}'
square_bracket ::= '[' | ']'
enclosure ::= parenthesis | bracket | square_bracket
infix_op ::= '+' | '-' | '*' | '/' | '//' | '**'
        | '%' | '&&' | '||' | '^^' | '<' | '<=' | '>' | '>='
        | 'and' | 'or' | 'is' | 'as' | 'isnot' | 'in' | 'notin' | 'dot' | 'cross'
prefix_op ::= '+' | '-' | '*' | '**' | '..' | '..<' | '~' | '&' | '!'
postfix_op ::= '?' | '..' | '<..'
operator ::= infix_op | prefix_op | postfix_op
char ::= /* ... */
str ::= '\"' char* '\"
symbol_head ::= /* char except dight */
symbol ::= symbol_head /* char except (reserved_symbol | operator | escape | ' ') */
subscript ::= accessor '[' expr ']'
attr ::= accessor '.' symbol
accessor ::= symbol | attr | subscript
literal ::= int | ratio | str | bool | none | ellipsis | not_implemented
pos_arg ::= expr
kw_arg ::= symbol ':' expr
arg ::= pos_arg | kw_arg
enc_args ::= pos_arg (',' pos_arg)* ','?
args ::= '()' | '(' arg (',' arg)* ','? ')' | arg (',' arg)*
var_pattern ::= accessor | `...` accessor | '[' single_patterns ']'
var_decl_opt_t = var_pattern (':' type)?
var_decl = var_pattern ':' type
param_pattern ::= symbol | `...` symbol | literal | '[' param_patterns ']'
param_decl_opt_t = param_pattern (':' type)?
param_decl = param_pattern ':' type
params_opt_t ::= '()' (':' type)?
        | '(' param_decl_opt_t (',' param_decl_opt_t)* ','? ')' (':' type)?
        | param_decl_opt_t (',' param_decl_opt_t)*
params ::= '()' ':' type
        | '(' param_decl (',' param_decl)* ','? ')' ':' type
subr_decl ::= accessor params
subr_decl_opt_t ::= accessor params_opt_t
decl ::= var_decl | subr_decl
decl_opt_t = var_decl_opt_t | subr_decl_opt_t
body ::= expr | indent line+ dedent
def ::= ('@' decorator '\n')* decl_opt_t '=' body
call ::= accessor args | accessor call
decorator ::= call
lambda_func ::= params_opt_t '->' body
lambda_proc ::= params_opt_t '=>' body
lambda ::= lambda_func | lambda_proc
normal_array ::= '[' enc_args ']'
array_comprehension ::= '[' expr | (generator)+ ']'
array ::= normal_array | array_comprehension
record ::= '{' '=' '}'
    | '{' def (';' def)* ';'? '}'
set ::= '{' '}'
    | '{' expr (',' expr)* ','? '}'
dict ::= '{' ':' '}'
    | '{' expr ':' expr (',' expr ':' expr)* ','? '}'
tuple ::= '(' ')'
    | '(' expr (',' expr)* ','? ')'
indent ::= /* ... */
expr ::= accessor | literal
    | prefix | infix | postfix
    | array | record | set | dict | tuple
    | call | def | lambda
line ::= expr separator+
program ::= expr? | (line | comment)*
```