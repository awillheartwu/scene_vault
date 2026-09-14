# 可视化检查脚本

jsdom 不计算布局，所以下面这些只在真实浏览器里才能发现的问题用脚本兜住。它们不参与
CI（需要 dev server 与 Chromium），在改动相关界面后手动跑一次。

## popup-layout.mjs

检查分类弹窗（人物模式）：

- 四种窗口尺寸（1115×940、880×620、660×640、560×480）下角色列表都至少能看到两行
  （最小尺寸允许滚动主体后到达，列表本身不小于 120px）；
- 每个尺寸下点「框选主脸」后，图片高度至少占窗口高度减 140px（即只剩标题栏与提示条），
  且不低于窗口高度的 70%，同时角色列表必须收起；
- 每个尺寸下取消框选后列表恢复到至少两行。

准备：先起前端 dev server，并准备 Playwright + Chromium（本机已有的路径如下）。

```bash
cd /tmp/sv-ui-check && node node_modules/vite/bin/vite.js --host 127.0.0.1 --port 1437 &

PLAYWRIGHT_MODULE=/mnt/c/Users/WuHaoli/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright/index.mjs \
CHROME_PATH=/home/loica/.cache/ms-playwright/chromium-1187/chrome-linux/chrome \
node scripts/visual-checks/popup-layout.mjs
```

输出每个尺寸的实测数值，全部通过时打印 `POPUP LAYOUT OK`，否则列出失败项并以非零码退出。
