# 离线渲染性能复验

## 变更

`preview_host.exe --render` 的解码槽原来按图层下标绑定。时间线上图层顺序变化时，同一个媒体路径会被重新打开。现在保留固定数量的解码槽，并在每帧按路径分配；同一路径的重复图层仍拿到不同槽位，避免覆盖另一个图层持有的 GPU 帧租约。槽位不足时按最近最少使用回收，因此不会随着项目中媒体路径数量增加而无限增长。

离线协议的逐帧读写仍保持同步。先测量再决定是否增加跨帧流水线：当前读回路径已经达到目标，暂不引入会改变二进制协议和取消语义的异步写入。

## 可复现命令

```powershell
cmd /c native\build.cmd
python native/tests/benchmark_render.py `
  --host native/build/bin/preview_host.exe `
  --width 1920 --height 1080 --layers 3 --frames 30 `
  --output target/render-benchmark.json
```

脚本生成无外部依赖的 1920×1080 PNG 夹具，连续提交 30 帧并逐帧读取完整 BGRA 缓冲，计时包含解码器打开、GPU 合成和像素读回。构建产物 SHA-256 为 `3d3e33da216465025b59b39464441a3e7a852b2dfebf81f057c895106d2a3d38`，本次结果如下：

| 指标 | 结果 |
| --- | ---: |
| 图层 / 帧数 | 3 / 30 |
| 总耗时 | 0.675720 s |
| 吞吐 | 44.397 fps |
| 30 fps 实时倍数 | 1.480× |
| v3 目标 | 0.8× |

因此 X5 的性能门槛在当前夹具上通过。该夹具覆盖合成和读回成本，真实视频解码仍应在最终验收的 3 层工程上再次记录；如果那组测量跌破 0.8×，再实现 `OfflineRenderer` 的跨帧流水线。

## 验证

- `cmd /c native\build.cmd` 通过。
- `cargo test -p todo_preview`：25 项通过。
- `python -m unittest discover -s native/tests -v`：26 项通过。
- `python native/tests/benchmark_render.py ...`：`passed: true`。
- `powershell -File build.ps1 -Configuration debug` 通过；`dist/to_do` 含最新宿主（SHA-256 与上方一致）、`extensions/sdk` 和 bundled Python，扩展测试文件数为 0，未复制 `config/ai.json`。
- 缺失盘符路径会在进入 Media Foundation 前被拒绝，离线协议返回 `cannot open layer`，避免无效路径探测阻塞取消。
