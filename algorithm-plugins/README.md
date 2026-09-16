# Algorithm Plugin 预留

每个算法插件使用独立环境和版本化 manifest。计划中的调用形式：

```text
<plugin-venv>/python -m ct_worker run request.json
```

默认三维重建方法将是 FBP。材料分辨没有默认实现，必须由用户提供符合契约的插件后才能执行。V1 不运行任何重建或材料算法，也不创建占位计算结果。
