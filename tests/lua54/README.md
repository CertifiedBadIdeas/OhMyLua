# Эталонный интерпретатор Lua 5.4

Проверки исполняемого пути используют следующий интерпретатор:

- путь в Windows: `C:\msys64\usr\bin\lua.exe`;
- путь в MSYS2: `/usr/bin/lua.exe`;
- пакет MSYS2: `lua 5.4.8-1`;
- строка версии: `Lua 5.4.8  Copyright (C) 1994-2025 Lua.org, PUC-Rio`;
- `_VERSION`: `Lua 5.4`;
- диапазон целых чисел: от `-9223372036854775808` до `9223372036854775807` (64 бита);
- `math.type(1)`: `integer`;
- `math.type(1.0)`: `float`;
- SHA-256 `lua.exe`: `c57a55e91860cf42b9876fe97401b9f7ed4bf1d5b38bfa1266ed3144b235cc58`.

Данные получены следующими командами без автоматической подмены интерпретатора:

```sh
lua -v
lua -e 'print(_VERSION)'
lua -e 'print(math.mininteger, math.maxinteger)'
lua -e 'print(math.type(1), math.type(1.0))'
pacman -Q lua
sha256sum /usr/bin/lua.exe
```

`run_and_print.lua` загружает переданный файл, исполняет его и печатает возвращённое значение.
