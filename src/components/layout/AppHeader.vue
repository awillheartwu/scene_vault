<template>
  <header class="app-header">
    <span class="app-header-mark">SV</span>
    <span class="app-header-title">Scene Vault</span>
    <span class="app-header-hint">本地优先 · 截图采集工作台</span>
  </header>
</template>

<style scoped>
.app-header {
  display: flex;
  height: 52px;
  flex: none;
  align-items: center;
  gap: 10px;
  padding: 0 24px;
  border-bottom: 1px solid color-mix(in srgb, var(--border) 72%, transparent);
  background: color-mix(in srgb, var(--background) 66%, transparent);
  backdrop-filter: var(--panel-blur);
  box-shadow: var(--inner-highlight), 0 6px 20px -16px rgba(0, 0, 0, 0.3);
}

.app-header-mark {
  display: grid;
  width: 26px;
  height: 26px;
  place-items: center;
  border-radius: 8px;
  background: var(--accent-gradient);
  color: var(--accent-foreground);
  font-size: 11px;
  font-weight: 800;
  letter-spacing: 0.04em;
  box-shadow: var(--glow);
}

.app-header-title {
  font-size: 13px;
  font-weight: 700;
  letter-spacing: 0.02em;
}

.app-header-hint {
  margin-left: auto;
  color: var(--muted-foreground);
  font-size: 11px;
}
</style>
