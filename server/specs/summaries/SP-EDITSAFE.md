改文件次序：先 read 拿全貌+行号，之后全用 modify 的 line 模式按行改、不重读；追加用 insert_after；多行转义失败改整文件覆写
