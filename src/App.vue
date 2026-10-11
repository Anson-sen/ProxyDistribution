<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";

interface ProxyNode {
  name: string;
  type: string;
  server: string;
  port: number;
  delay?: number | null; // null: untested, -999: testing, -1: timeout, >0: ms
  raw: any;
}

interface PortMapping {
  port: number;
  node_name: string;
  node_type: string;
  server: string;
  proxy_type: string;
  listen_ip: string;
}

interface PoolStatus {
  running: boolean;
  pid?: number | null;
  port_count: number;
  controller_port: number;
  port_mappings: PortMapping[];
}

// ==================== THEME (FOLLOW SYSTEM BY DEFAULT) ====================
type ThemeMode = "system" | "dark" | "light";
const themeMode = ref<ThemeMode>(
  (localStorage.getItem("proxy_theme_mode") as ThemeMode) || "system"
);
const systemPrefersDark = ref(
  window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches
);

let mediaQuery: MediaQueryList | null = null;
function handleSystemThemeChange(e: MediaQueryListEvent) {
  systemPrefersDark.value = e.matches;
}

const currentTheme = computed<"dark" | "light">(() => {
  if (themeMode.value === "system") {
    return systemPrefersDark.value ? "dark" : "light";
  }
  return themeMode.value;
});

function cycleTheme() {
  if (themeMode.value === "system") {
    themeMode.value = "dark";
  } else if (themeMode.value === "dark") {
    themeMode.value = "light";
  } else {
    themeMode.value = "system";
  }
  localStorage.setItem("proxy_theme_mode", themeMode.value);
  showToast(`主题模式: ${themeModeLabel.value}`);
}

const themeModeLabel = computed(() => {
  if (themeMode.value === "system") return "跟随系统";
  if (themeMode.value === "dark") return "深色模式";
  return "浅色模式";
});

// ==================== STATE & PERSISTENCE ====================
const subUrl = ref("");
const rawText = ref("");
const importMode = ref<"url" | "file" | "text">("url");
const startPort = ref(10001);
const proxyType = ref<"mixed" | "socks">("mixed");
const listenIp = ref<"127.0.0.1" | "0.0.0.0">("127.0.0.1");
const autoFilterTimeout = ref(true);

const nodes = ref<ProxyNode[]>([]);
const selectedNodeNames = ref<Set<string>>(new Set());

const poolStatus = ref<PoolStatus>({
  running: false,
  port_count: 0,
  controller_port: 19090,
  port_mappings: [],
});

const searchQuery = ref("");
const selectedType = ref("ALL");
const latencyFilter = ref<"ALL" | "ALIVE" | "TIMEOUT">("ALL");

const testingAll = ref(false);
const loading = ref(false);
const toastMsg = ref("");
const showExportModal = ref(false);
const exportFormat = ref<"plain" | "socks5" | "http" | "csv">("plain");
const fileInputRef = ref<HTMLInputElement | null>(null);
const isDragging = ref(false);

// Toast helper
let toastTimer: any = null;
function showToast(msg: string) {
  toastMsg.value = msg;
  if (toastTimer) clearTimeout(toastTimer);
  toastTimer = setTimeout(() => {
    toastMsg.value = "";
  }, 2500);
}

// Copy helper
async function copyToClipboard(text: string, tip = "已复制到剪贴板") {
  try {
    await navigator.clipboard.writeText(text);
    showToast(tip);
  } catch {
    showToast("复制失败，请手动选择复制");
  }
}

// Persistence: Save nodes and config
function saveToStorage() {
  try {
    localStorage.setItem("saved_nodes", JSON.stringify(nodes.value));
    localStorage.setItem("saved_sub_url", subUrl.value);
    localStorage.setItem("saved_start_port", String(startPort.value));
    localStorage.setItem("saved_proxy_type", proxyType.value);
    localStorage.setItem("saved_listen_ip", listenIp.value);
    localStorage.setItem("saved_auto_filter", String(autoFilterTimeout.value));
  } catch (err) {
    console.error("Failed to save state", err);
  }
}

// Persistence: Load nodes and config
function loadFromStorage() {
  try {
    const savedNodes = localStorage.getItem("saved_nodes");
    if (savedNodes) {
      const parsed = JSON.parse(savedNodes);
      if (Array.isArray(parsed) && parsed.length > 0) {
        nodes.value = parsed;
      }
    }
    const savedUrl = localStorage.getItem("saved_sub_url");
    if (savedUrl) subUrl.value = savedUrl;
    const savedPort = localStorage.getItem("saved_start_port");
    if (savedPort) startPort.value = Number(savedPort);
    const savedType = localStorage.getItem("saved_proxy_type");
    if (savedType) proxyType.value = savedType as any;
    const savedIp = localStorage.getItem("saved_listen_ip");
    if (savedIp) listenIp.value = savedIp as any;
    const savedAuto = localStorage.getItem("saved_auto_filter");
    if (savedAuto) autoFilterTimeout.value = savedAuto === "true";
  } catch (err) {
    console.error("Failed to load state", err);
  }
}

// Auto save whenever nodes or settings change
watch(
  [nodes, subUrl, startPort, proxyType, listenIp, autoFilterTimeout],
  () => {
    saveToStorage();
  },
  { deep: true }
);

// Fetch or parse nodes
async function handleImport() {
  loading.value = true;
  try {
    if (importMode.value === "url") {
      if (!subUrl.value.trim()) {
        showToast("请输入订阅 URL");
        return;
      }
      const parsed = await invoke<ProxyNode[]>("fetch_subscription", {
        url: subUrl.value.trim(),
      });
      nodes.value = parsed;
      selectedNodeNames.value.clear();
      saveToStorage();
      showToast(`成功解析 ${parsed.length} 个节点并自动保存`);
    } else if (importMode.value === "text") {
      if (!rawText.value.trim()) {
        showToast("请输入节点文本或订阅内容");
        return;
      }
      const parsed = await invoke<ProxyNode[]>("parse_text", {
        content: rawText.value.trim(),
      });
      nodes.value = parsed;
      selectedNodeNames.value.clear();
      saveToStorage();
      showToast(`成功解析 ${parsed.length} 个节点并自动保存`);
    }
  } catch (err: any) {
    showToast(`导入失败: ${err}`);
  } finally {
    loading.value = false;
  }
}

// File Import Handler (YAML / YML / TXT)
function triggerFileInput() {
  fileInputRef.value?.click();
}

function handleFileChange(e: Event) {
  const target = e.target as HTMLInputElement;
  const file = target.files?.[0];
  if (file) {
    parseSelectedFile(file);
  }
}

function handleFileDrop(e: DragEvent) {
  isDragging.value = false;
  const file = e.dataTransfer?.files?.[0];
  if (file) {
    parseSelectedFile(file);
  }
}

async function parseSelectedFile(file: File) {
  loading.value = true;
  try {
    const reader = new FileReader();
    reader.onload = async (event) => {
      const content = event.target?.result as string;
      if (!content || !content.trim()) {
        showToast("所选文件内容为空");
        loading.value = false;
        return;
      }
      try {
        const parsed = await invoke<ProxyNode[]>("parse_text", {
          content: content.trim(),
        });
        nodes.value = parsed;
        selectedNodeNames.value.clear();
        saveToStorage();
        showToast(`从文件【${file.name}】成功导入 ${parsed.length} 个节点`);
      } catch (err: any) {
        showToast(`文件解析失败: ${err}`);
      } finally {
        loading.value = false;
      }
    };
    reader.onerror = () => {
      showToast("读取文件发生错误");
      loading.value = false;
    };
    reader.readAsText(file, "UTF-8");
  } catch (err: any) {
    showToast(`处理文件失败: ${err}`);
    loading.value = false;
  }
}

async function parseFilePath(filePath: string) {
  loading.value = true;
  try {
    const parsed = await invoke<ProxyNode[]>("read_file_and_parse", {
      filePath: filePath,
    });
    nodes.value = parsed;
    selectedNodeNames.value.clear();
    saveToStorage();
    const filename = filePath.split(/[/\\]/).pop() || filePath;
    showToast(`成功从文件【${filename}】导入 ${parsed.length} 个节点并自动保存`);
  } catch (err: any) {
    showToast(`文件导入失败: ${err}`);
  } finally {
    loading.value = false;
  }
}

// Single delay test
async function testDelay(node: ProxyNode) {
  node.delay = -999;
  try {
    const rawProxies = [node.raw];
    const delay = await invoke<number>("test_single_delay", {
      proxies: rawProxies,
      nodeName: node.name,
    });
    node.delay = delay;
    saveToStorage();
  } catch {
    node.delay = -1;
    saveToStorage();
  }
}

// Batch test all delays
async function testAllDelays() {
  if (nodes.value.length === 0) {
    showToast("当前没有节点可供测速");
    return;
  }
  if (testingAll.value) return;

  testingAll.value = true;
  nodes.value.forEach((n) => (n.delay = -999));
  showToast(poolStatus.value.running ? "正在测速中..." : "正在进行离线快速测速...");

  try {
    const rawProxies = nodes.value.map((n) => n.raw);
    const names = nodes.value.map((n) => n.name);
    const results = await invoke<Record<string, number>>("test_all_delays", {
      proxies: rawProxies,
      nodes: names,
    });
    for (const n of nodes.value) {
      if (results[n.name] !== undefined) {
        n.delay = results[n.name];
      }
    }
    saveToStorage();
    showToast("批量测速完成！");
  } catch (err: any) {
    showToast(`测速出错: ${err}`);
  } finally {
    testingAll.value = false;
  }
}

// Remove timeout nodes
function removeTimeoutNodes() {
  const beforeCount = nodes.value.length;
  nodes.value = nodes.value.filter((n) => n.delay !== -1);
  const removed = beforeCount - nodes.value.length;
  saveToStorage();
  if (removed > 0) {
    showToast(`已排除 ${removed} 个超时节点，剩余 ${nodes.value.length} 个节点`);
  } else {
    showToast("当前没有超时节点");
  }
}

// Batch delete selected nodes
function deleteSelectedNodes() {
  if (selectedNodeNames.value.size === 0) {
    showToast("未勾选任何节点");
    return;
  }
  const count = selectedNodeNames.value.size;
  nodes.value = nodes.value.filter((n) => !selectedNodeNames.value.has(n.name));
  selectedNodeNames.value.clear();
  saveToStorage();
  showToast(`已删除 ${count} 个选中节点`);
}

// Selection helpers
function toggleSelectAll() {
  if (selectedNodeNames.value.size === filteredNodes.value.length) {
    selectedNodeNames.value.clear();
  } else {
    selectedNodeNames.value = new Set(filteredNodes.value.map((n) => n.name));
  }
}

function deleteSingleNode(name: string) {
  nodes.value = nodes.value.filter((n) => n.name !== name);
  selectedNodeNames.value.delete(name);
  saveToStorage();
}

// Clear all nodes
function clearAllNodes() {
  if (nodes.value.length === 0) return;
  if (confirm("确定要清空当前所有导入的节点吗？")) {
    nodes.value = [];
    selectedNodeNames.value.clear();
    saveToStorage();
    showToast("已清空节点列表");
  }
}

// Start / Stop Proxy Pool
async function togglePool() {
  loading.value = true;
  try {
    if (poolStatus.value.running) {
      await invoke("stop_pool");
      poolStatus.value = await invoke<PoolStatus>("get_status");
      showToast("代理池已停止");
    } else {
      if (nodes.value.length === 0) {
        showToast("当前没有节点，请先导入订阅或文件");
        return;
      }

      let targetNodes = nodes.value;
      if (autoFilterTimeout.value) {
        const aliveOrUntested = nodes.value.filter((n) => n.delay !== -1);
        if (aliveOrUntested.length > 0) {
          targetNodes = aliveOrUntested;
        }
      }

      const rawProxies = targetNodes.map((n) => n.raw);
      const res = await invoke<PoolStatus>("start_pool", {
        req: {
          raw_proxies: rawProxies,
          start_port: Number(startPort.value),
          proxy_type: proxyType.value,
          listen_ip: listenIp.value,
        },
      });
      poolStatus.value = res;
      showToast(`代理池已启动！已开放 ${res.port_count} 个独立本地端口`);
    }
  } catch (err: any) {
    showToast(`操作失败: ${err}`);
  } finally {
    loading.value = false;
  }
}

// Check pool status periodically
let statusInterval: any = null;
async function refreshStatus() {
  try {
    poolStatus.value = await invoke<PoolStatus>("get_status");
  } catch {}
}

let unlistenDragDrop: (() => void) | null = null;

onMounted(async () => {
  loadFromStorage();
  refreshStatus();
  statusInterval = setInterval(refreshStatus, 3000);

  if (window.matchMedia) {
    mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
    mediaQuery.addEventListener("change", handleSystemThemeChange);
  }

  // Tauri v2 native file drag & drop listener
  try {
    const webview = getCurrentWebview();
    unlistenDragDrop = await webview.onDragDropEvent(async (event) => {
      if (event.payload.type === "enter" || event.payload.type === "over") {
        isDragging.value = true;
      } else if (event.payload.type === "leave") {
        isDragging.value = false;
      } else if (event.payload.type === "drop") {
        isDragging.value = false;
        const paths = event.payload.paths;
        if (paths && paths.length > 0) {
          await parseFilePath(paths[0]);
        }
      }
    });
  } catch (err) {
    console.error("Tauri drag-drop init failed:", err);
  }
});

onUnmounted(() => {
  if (statusInterval) clearInterval(statusInterval);
  if (mediaQuery) {
    mediaQuery.removeEventListener("change", handleSystemThemeChange);
  }
  if (unlistenDragDrop) {
    unlistenDragDrop();
  }
});

// Map node to port
const nodePortMap = computed(() => {
  const map: Record<string, number> = {};
  for (const m of poolStatus.value.port_mappings) {
    map[m.node_name] = m.port;
  }
  return map;
});

// Statistics
const stats = computed(() => {
  const total = nodes.value.length;
  let alive = 0;
  let timeout = 0;
  let untested = 0;
  nodes.value.forEach((n) => {
    if (n.delay === -1) timeout++;
    else if (n.delay !== undefined && n.delay !== null && n.delay >= 0) alive++;
    else untested++;
  });
  return { total, alive, timeout, untested };
});

// Filtered nodes
const availableProtocols = computed(() => {
  const set = new Set<string>();
  nodes.value.forEach((n) => set.add(n.type.toUpperCase()));
  return Array.from(set);
});

const filteredNodes = computed(() => {
  return nodes.value.filter((n) => {
    const matchesSearch =
      n.name.toLowerCase().includes(searchQuery.value.toLowerCase()) ||
      n.server.toLowerCase().includes(searchQuery.value.toLowerCase());
    const matchesType =
      selectedType.value === "ALL" ||
      n.type.toUpperCase() === selectedType.value;
    const matchesLatency =
      latencyFilter.value === "ALL" ||
      (latencyFilter.value === "ALIVE" && n.delay !== undefined && n.delay !== null && n.delay >= 0) ||
      (latencyFilter.value === "TIMEOUT" && n.delay === -1);

    return matchesSearch && matchesType && matchesLatency;
  });
});

const isAllFilteredSelected = computed(() => {
  if (filteredNodes.value.length === 0) return false;
  return filteredNodes.value.every((n) => selectedNodeNames.value.has(n.name));
});

// Export text generation
const exportContent = computed(() => {
  if (!poolStatus.value.running || poolStatus.value.port_mappings.length === 0) {
    return "代理池尚未启动，暂无可用端口。";
  }
  const lines: string[] = [];
  const ip = listenIp.value === "0.0.0.0" ? "127.0.0.1" : listenIp.value;

  if (exportFormat.value === "csv") {
    lines.push("Port,NodeName,Type,Server,Delay(ms)");
    for (const m of poolStatus.value.port_mappings) {
      const node = nodes.value.find((n) => n.name === m.node_name);
      const delay = node?.delay !== undefined && node?.delay !== null ? node.delay : "-";
      lines.push(`${m.port},"${m.node_name}",${m.node_type},${m.server},${delay}`);
    }
  } else if (exportFormat.value === "socks5") {
    for (const m of poolStatus.value.port_mappings) {
      lines.push(`socks5://${ip}:${m.port}`);
    }
  } else if (exportFormat.value === "http") {
    for (const m of poolStatus.value.port_mappings) {
      lines.push(`http://${ip}:${m.port}`);
    }
  } else {
    for (const m of poolStatus.value.port_mappings) {
      lines.push(`${ip}:${m.port}`);
    }
  }

  return lines.join("\n");
});
</script>

<template>
  <div class="app-layout" :class="'theme-' + currentTheme">
    <!-- Global File Drag & Drop Visual Overlay -->
    <transition name="fade">
      <div v-if="isDragging" class="global-drag-overlay">
        <div class="drag-box">
          <div class="drag-icon-large">📥</div>
          <h2>松开鼠标即可导入文件</h2>
          <p>支持拖入 .yaml / .yml / .txt 配置文件与节点文本</p>
        </div>
      </div>
    </transition>

    <!-- Hidden File Input for YAML/TXT -->
    <input
      ref="fileInputRef"
      type="file"
      accept=".yaml,.yml,.txt"
      style="display: none"
      @change="handleFileChange"
    />

    <!-- ==================== LEFT SIDEBAR ==================== -->
    <aside class="sidebar">
      <!-- Brand & Theme Row -->
      <div class="sidebar-top">
        <div class="brand">
          <div class="brand-logo">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" class="brand-icon">
              <path d="M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5" />
            </svg>
          </div>
          <div>
            <h1 class="brand-name">ProxyDistribution</h1>
            <span class="brand-desc">多端口节点分发池 · Mihomo</span>
          </div>
        </div>

        <!-- Theme Switcher (System / Dark / Light) -->
        <button class="theme-btn" :title="'当前: ' + themeModeLabel + ' (点击切换)'" @click="cycleTheme">
          <span v-if="themeMode === 'system'">🌓 自动</span>
          <span v-else-if="themeMode === 'dark'">🌙 深色</span>
          <span v-else>☀️ 浅色</span>
        </button>
      </div>

      <!-- Stats Summary Card -->
      <div class="side-card stats-card">
        <div class="stats-grid">
          <div class="stat-box">
            <span class="stat-num">{{ stats.total }}</span>
            <span class="stat-txt">总节点</span>
          </div>
          <div class="stat-box stat-alive">
            <span class="stat-num">{{ stats.alive }}</span>
            <span class="stat-txt">可用</span>
          </div>
          <div class="stat-box stat-timeout">
            <span class="stat-num">{{ stats.timeout }}</span>
            <span class="stat-txt">超时</span>
          </div>
          <div class="stat-box stat-ports">
            <span class="stat-num">{{ poolStatus.port_count }}</span>
            <span class="stat-txt">端口数</span>
          </div>
        </div>
      </div>

      <!-- Import Section -->
      <div class="side-card import-card">
        <div class="side-card-header">
          <span class="section-title">📥 节点导入</span>
          <button v-if="nodes.length > 0" class="btn-clear" title="清空全部节点" @click="clearAllNodes">
            清空
          </button>
        </div>

        <!-- Import Tabs -->
        <div class="tab-switch">
          <button :class="{ active: importMode === 'url' }" @click="importMode = 'url'">
            🔗 订阅链接
          </button>
          <button :class="{ active: importMode === 'file' }" @click="importMode = 'file'">
            📁 导入 YAML
          </button>
          <button :class="{ active: importMode === 'text' }" @click="importMode = 'text'">
            📝 文本粘贴
          </button>
        </div>

        <!-- Tab 1: URL -->
        <div v-if="importMode === 'url'" class="import-panel">
          <div class="url-input-group">
            <label class="url-label">订阅地址 URL</label>
            <input
              v-model="subUrl"
              class="side-input"
              placeholder="粘贴机场订阅 URL (https://...)"
              @keyup.enter="handleImport"
            />
            <div class="url-tip-box">
              <span class="tip-icon">💡</span>
              <span>支持各大机场 Clash、Mihomo、Base64 订阅。导入后自动本地持久化。</span>
            </div>
          </div>
          <button class="btn btn-primary btn-block" :disabled="loading" @click="handleImport">
            {{ loading ? '解析中...' : '拉取并解析订阅' }}
          </button>
        </div>

        <!-- Tab 2: File Upload (YAML / YML) -->
        <div v-else-if="importMode === 'file'" class="import-panel">
          <div
            class="drop-zone"
            :class="{ 'is-dragging': isDragging }"
            @click="triggerFileInput"
            @dragover.prevent="isDragging = true"
            @dragleave.prevent="isDragging = false"
            @drop.prevent="handleFileDrop"
          >
            <div class="drop-icon">📁</div>
            <p class="drop-text">点击选择或拖拽 <strong>.yaml / .yml</strong> 到此处</p>
            <span class="drop-hint">支持 Clash / Mihomo 配置文件与节点文本</span>
          </div>
        </div>

        <!-- Tab 3: Text Paste -->
        <div v-else class="import-panel">
          <textarea
            v-model="rawText"
            class="side-textarea"
            placeholder="粘贴 Clash YAML、Base64 或节点链接 (vmess://, vless://, ss://, hy2://)..."
          ></textarea>
          <button class="btn btn-primary btn-block" :disabled="loading" @click="handleImport">
            {{ loading ? '解析中...' : '解析文本节点' }}
          </button>
        </div>
      </div>

      <!-- Settings Section -->
      <div class="side-card settings-card">
        <span class="section-title">⚙️ 调度配置</span>

        <div class="setting-row">
          <label>起始端口</label>
          <input
            v-model.number="startPort"
            type="number"
            class="setting-field-ctrl"
            :disabled="poolStatus.running"
            min="1024"
            max="65000"
          />
        </div>

        <div class="setting-row">
          <label>代理模式</label>
          <select v-model="proxyType" class="setting-field-ctrl" :disabled="poolStatus.running">
            <option value="mixed">Mixed (SOCKS5 + HTTP)</option>
            <option value="socks">纯 SOCKS5</option>
          </select>
        </div>

        <div class="setting-row">
          <label>监听地址</label>
          <select v-model="listenIp" class="setting-field-ctrl" :disabled="poolStatus.running">
            <option value="127.0.0.1">127.0.0.1 (仅本机)</option>
            <option value="0.0.0.0">0.0.0.0 (局域网共享)</option>
          </select>
        </div>

        <label class="auto-filter-check" title="启动时自动跳过超时的节点，不为其分配端口">
          <input v-model="autoFilterTimeout" type="checkbox" />
          <span>启动时自动排除超时节点</span>
        </label>
      </div>

      <!-- Master Power Button (Sidebar Bottom) -->
      <div class="sidebar-bottom">
        <button
          class="btn-power-main"
          :class="poolStatus.running ? 'power-running' : 'power-stopped'"
          :disabled="loading"
          @click="togglePool"
        >
          <span v-if="loading" class="spinner"></span>
          <span v-else class="power-dot"></span>
          <span>{{ poolStatus.running ? '停止代理池' : '启动代理池' }}</span>
        </button>

        <span class="persistence-hint">💾 节点与配置已自动本地保存</span>
      </div>
    </aside>

    <!-- ==================== RIGHT MAIN CONTENT ==================== -->
    <main class="main-content">
      <!-- Top Action & Filter Header (Spacious 2-Tier Layout) -->
      <header class="main-header">
        <!-- Tier 1: Search & Primary Action Buttons -->
        <div class="toolbar-top">
          <div class="toolbar-search">
            <svg class="search-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <circle cx="11" cy="11" r="8"></circle>
              <line x1="21" y1="21" x2="16.65" y2="16.65"></line>
            </svg>
            <input
              v-model="searchQuery"
              class="search-input"
              placeholder="搜索节点名称、域名或 IP 地址..."
            />
            <button
              v-if="searchQuery"
              class="search-clear-btn"
              title="清空搜索"
              @click="searchQuery = ''"
            >
              ✕
            </button>
          </div>

          <!-- Action Buttons -->
          <div class="action-btns">
            <button
              class="btn btn-test"
              :disabled="testingAll || nodes.length === 0"
              title="测速全部节点延迟（支持启动前及运行中）"
              @click="testAllDelays"
            >
              <span v-if="testingAll" class="spinner"></span>
              <span v-else>⚡</span>
              <span>测速全部</span>
            </button>

            <button
              v-if="stats.timeout > 0"
              class="btn btn-warn"
              title="一键剔除列表中所有超时的失效节点"
              @click="removeTimeoutNodes"
            >
              <span>🧹</span>
              <span>排除超时 ({{ stats.timeout }})</span>
            </button>

            <button
              v-if="selectedNodeNames.size > 0"
              class="btn btn-danger"
              title="批量删除已选中的节点"
              @click="deleteSelectedNodes"
            >
              <span>🗑️</span>
              <span>删除选中 ({{ selectedNodeNames.size }})</span>
            </button>

            <button
              class="btn btn-accent"
              :disabled="!poolStatus.running"
              :title="poolStatus.running ? '一键导出所有已分配的代理端口' : '需先启动代理池以生成可用端口'"
              @click="showExportModal = true"
            >
              <span>📋</span>
              <span>一键导出端口</span>
            </button>
          </div>
        </div>

        <!-- Tier 2: Category & Latency Filters -->
        <div class="toolbar-bottom">
          <div class="filter-section">
            <div class="filter-group">
              <span class="filter-label">协议:</span>
              <button
                class="pill-tag"
                :class="{ active: selectedType === 'ALL' }"
                @click="selectedType = 'ALL'"
              >
                全部
              </button>
              <button
                v-for="proto in availableProtocols"
                :key="proto"
                class="pill-tag"
                :class="{ active: selectedType === proto }"
                @click="selectedType = proto"
              >
                {{ proto }}
              </button>
            </div>

            <div class="filter-divider"></div>

            <div class="filter-group">
              <span class="filter-label">状态:</span>
              <button
                class="pill-tag"
                :class="{ active: latencyFilter === 'ALL' }"
                @click="latencyFilter = 'ALL'"
              >
                全部
              </button>
              <button
                class="pill-tag pill-green"
                :class="{ active: latencyFilter === 'ALIVE' }"
                @click="latencyFilter = 'ALIVE'"
              >
                可用 ({{ stats.alive }})
              </button>
              <button
                class="pill-tag pill-red"
                :class="{ active: latencyFilter === 'TIMEOUT' }"
                @click="latencyFilter = 'TIMEOUT'"
              >
                超时 ({{ stats.timeout }})
              </button>
            </div>
          </div>

          <div class="node-count-badge">
            已显示 <strong>{{ filteredNodes.length }}</strong> / 共 {{ nodes.length }} 节点
          </div>
        </div>
      </header>

      <!-- Table Scroll Area (STRICT INTERNAL SCROLL) -->
      <section class="table-area">
        <table class="data-table">
          <thead>
            <tr>
              <th style="width: 40px; min-width: 40px; text-align: center">
                <input
                  type="checkbox"
                  :checked="isAllFilteredSelected"
                  @change="toggleSelectAll"
                />
              </th>
              <th style="width: 44px; min-width: 44px; text-align: center">#</th>
              <th style="width: 100px; min-width: 100px; text-align: center">本地端口</th>
              <th style="min-width: 180px">节点名称</th>
              <th style="width: 85px; min-width: 85px; text-align: center">协议</th>
              <th style="min-width: 180px">远端服务器</th>
              <th style="width: 95px; min-width: 95px; text-align: center">延迟</th>
              <th style="width: 200px; min-width: 180px; text-align: right; padding-right: 14px">快捷操作</th>
            </tr>
          </thead>
          <tbody>
            <tr v-if="filteredNodes.length === 0">
              <td colspan="8" class="table-empty">
                <div class="empty-icon">📡</div>
                <p>{{ nodes.length === 0 ? '暂无节点，请在左侧导入订阅、选择 YAML 文件或粘贴文本' : '未找到匹配的节点' }}</p>
              </td>
            </tr>

            <tr
              v-for="(node, idx) in filteredNodes"
              :key="node.name"
              :class="{ 'row-active': selectedNodeNames.has(node.name) }"
            >
              <!-- Checkbox -->
              <td style="text-align: center">
                <input
                  type="checkbox"
                  :checked="selectedNodeNames.has(node.name)"
                  @change="selectedNodeNames.has(node.name) ? selectedNodeNames.delete(node.name) : selectedNodeNames.add(node.name)"
                />
              </td>

              <!-- Index -->
              <td class="dim-text" style="text-align: center">{{ idx + 1 }}</td>

              <!-- Port Badge -->
              <td style="text-align: center">
                <span
                  v-if="nodePortMap[node.name]"
                  class="port-tag clickable"
                  title="点击复制 127.0.0.1:端口"
                  @click="copyToClipboard(`${listenIp === '0.0.0.0' ? '127.0.0.1' : listenIp}:${nodePortMap[node.name]}`, `已复制端口 ${nodePortMap[node.name]}`)"
                >
                  {{ nodePortMap[node.name] }}
                </span>
                <span v-else class="dim-text">--</span>
              </td>

              <!-- Node Name -->
              <td class="node-cell" :title="node.name">
                {{ node.name }}
              </td>

              <!-- Protocol -->
              <td style="text-align: center">
                <span class="proto-tag" :class="'proto-' + node.type.toLowerCase()">
                  {{ node.type.toUpperCase() }}
                </span>
              </td>

              <!-- Remote Server -->
              <td class="dim-text font-mono">{{ node.server }}:{{ node.port }}</td>

              <!-- Latency -->
              <td style="text-align: center">
                <span v-if="node.delay === -999" class="delay-tag delay-testing">
                  <span class="mini-spinner"></span> 测速中
                </span>
                <span
                  v-else-if="node.delay !== undefined && node.delay !== null && node.delay >= 0"
                  class="delay-tag"
                  :class="node.delay < 180 ? 'delay-green' : node.delay < 350 ? 'delay-yellow' : 'delay-orange'"
                >
                  {{ node.delay }} ms
                </span>
                <span
                  v-else-if="node.delay === -1"
                  class="delay-tag delay-red"
                >
                  超时
                </span>
                <span v-else class="dim-text">--</span>
              </td>

              <!-- Actions -->
              <td class="action-cell">
                <button
                  class="btn-row btn-row-speed"
                  title="测试当前节点延迟"
                  :disabled="node.delay === -999"
                  @click="testDelay(node)"
                >
                  <span v-if="node.delay === -999" class="mini-spinner"></span>
                  <span v-else>⚡ 测速</span>
                </button>
                <button
                  v-if="nodePortMap[node.name]"
                  class="btn-row"
                  title="复制 Socks5 代理地址"
                  @click="copyToClipboard(`socks5://${listenIp === '0.0.0.0' ? '127.0.0.1' : listenIp}:${nodePortMap[node.name]}`, '已复制 SOCKS5 地址')"
                >
                  Socks
                </button>
                <button
                  v-if="nodePortMap[node.name]"
                  class="btn-row"
                  title="复制 HTTP 代理地址"
                  @click="copyToClipboard(`http://${listenIp === '0.0.0.0' ? '127.0.0.1' : listenIp}:${nodePortMap[node.name]}`, '已复制 HTTP 地址')"
                >
                  HTTP
                </button>
                <button
                  class="btn-row btn-row-del"
                  title="删除此节点"
                  @click="deleteSingleNode(node.name)"
                >
                  ✕
                </button>
              </td>
            </tr>
          </tbody>
        </table>
      </section>
    </main>

    <!-- ==================== EXPORT MODAL ==================== -->
    <div v-if="showExportModal" class="modal-backdrop" @click.self="showExportModal = false">
      <div class="modal-card">
        <div class="modal-header">
          <h3>📋 批量导出端口表</h3>
          <button class="modal-close" @click="showExportModal = false">✕</button>
        </div>
        <div class="modal-body">
          <div class="radio-group">
            <label class="radio-label">
              <input v-model="exportFormat" type="radio" value="plain" />
              <span>IP:端口 (指纹浏览器常用)</span>
            </label>
            <label class="radio-label">
              <input v-model="exportFormat" type="radio" value="socks5" />
              <span>SOCKS5 链接</span>
            </label>
            <label class="radio-label">
              <input v-model="exportFormat" type="radio" value="http" />
              <span>HTTP 链接</span>
            </label>
            <label class="radio-label">
              <input v-model="exportFormat" type="radio" value="csv" />
              <span>CSV 表格</span>
            </label>
          </div>

          <textarea class="export-box" readonly :value="exportContent" rows="12"></textarea>
        </div>
        <div class="modal-footer">
          <button class="btn btn-secondary" @click="showExportModal = false">关闭</button>
          <button
            class="btn btn-primary"
            @click="copyToClipboard(exportContent, '已批量复制全部代理端口！')"
          >
            一键复制全部 ({{ poolStatus.port_count }}条)
          </button>
        </div>
      </div>
    </div>

    <!-- Toast Notification -->
    <transition name="toast-fade">
      <div v-if="toastMsg" class="toast-popup">
        {{ toastMsg }}
      </div>
    </transition>
  </div>
</template>

<style scoped>
/* ==================== THEME VARIABLES ==================== */
.theme-dark {
  --bg-app: #080c14;
  --bg-sidebar: #0f1624;
  --bg-card: #141c2c;
  --bg-card-sub: #0b111c;
  --bg-input: #090e18;
  --bg-table-head: #162032;
  --bg-table-hover: #131b2a;
  --bg-table-active: rgba(14, 165, 233, 0.09);
  --border-color: #1e293b;
  --border-input: #27354a;
  --text-main: #f8fafc;
  --text-sub: #94a3b8;
  --text-dim: #64748b;
  --scroll-track: #090d16;
  --scroll-thumb: #233046;
  --btn-sec-bg: #1e293b;
  --btn-sec-txt: #cbd5e1;
}

.theme-light {
  --bg-app: #f1f5f9;
  --bg-sidebar: #ffffff;
  --bg-card: #f8fafc;
  --bg-card-sub: #f1f5f9;
  --bg-input: #ffffff;
  --bg-table-head: #e2e8f0;
  --bg-table-hover: #f8fafc;
  --bg-table-active: rgba(2, 132, 199, 0.08);
  --border-color: #e2e8f0;
  --border-input: #cbd5e1;
  --text-main: #0f172a;
  --text-sub: #475569;
  --text-dim: #64748b;
  --scroll-track: #f1f5f9;
  --scroll-thumb: #cbd5e1;
  --btn-sec-bg: #e2e8f0;
  --btn-sec-txt: #334155;
}

/* ==================== LAYOUT (LEFT - RIGHT) ==================== */
.app-layout {
  display: flex;
  height: 100vh;
  width: 100vw;
  background-color: var(--bg-app);
  color: var(--text-main);
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "PingFang SC", "Hiragino Sans GB", "Microsoft YaHei", sans-serif;
  user-select: none;
  overflow: hidden; /* ABSOLUTELY NO GLOBAL WINDOW SCROLLBAR */
  box-sizing: border-box;
  transition: background-color 0.25s, color 0.25s;
}

/* ==================== LEFT SIDEBAR ==================== */
.sidebar {
  width: 330px;
  min-width: 330px;
  max-width: 330px;
  height: 100vh;
  background-color: var(--bg-sidebar);
  border-right: 1px solid var(--border-color);
  display: flex;
  flex-direction: column;
  padding: 14px 14px;
  gap: 12px;
  overflow-y: auto;
  box-sizing: border-box;
}

/* Custom scrollbar for sidebar */
.sidebar::-webkit-scrollbar {
  width: 4px;
}
.sidebar::-webkit-scrollbar-thumb {
  background: var(--scroll-thumb);
  border-radius: 2px;
}

.sidebar-top {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding-bottom: 4px;
}

.brand {
  display: flex;
  align-items: center;
  gap: 10px;
}

.brand-logo {
  width: 34px;
  height: 34px;
  border-radius: 8px;
  background: linear-gradient(135deg, #0284c7 0%, #06b6d4 100%);
  display: flex;
  align-items: center;
  justify-content: center;
  color: #fff;
  box-shadow: 0 0 12px rgba(6, 182, 212, 0.35);
}

.brand-icon {
  width: 20px;
  height: 20px;
}

.brand-name {
  margin: 0;
  font-size: 1.05rem;
  font-weight: 700;
  color: var(--text-main);
  letter-spacing: -0.02em;
}

.brand-desc {
  font-size: 0.68rem;
  color: var(--text-dim);
}

.theme-btn {
  background-color: var(--bg-card-sub);
  border: 1px solid var(--border-color);
  color: var(--text-main);
  font-size: 0.72rem;
  font-weight: 600;
  padding: 4px 8px;
  border-radius: 16px;
  cursor: pointer;
  transition: all 0.2s;
}

.theme-btn:hover {
  border-color: #0284c7;
  color: #0284c7;
}

/* Side Card */
.side-card {
  background-color: var(--bg-card);
  border: 1px solid var(--border-color);
  border-radius: 8px;
  padding: 10px 12px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.side-card-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.section-title {
  font-size: 0.78rem;
  font-weight: 700;
  color: var(--text-sub);
}

.btn-clear {
  background: transparent;
  border: none;
  font-size: 0.7rem;
  color: #f43f5e;
  cursor: pointer;
}

.btn-clear:hover {
  text-decoration: underline;
}

/* Stats Grid */
.stats-grid {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 6px;
}

.stat-box {
  background-color: var(--bg-card-sub);
  border: 1px solid var(--border-color);
  border-radius: 6px;
  padding: 6px 4px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 2px;
}

.stat-num {
  font-size: 0.92rem;
  font-weight: 700;
  color: var(--text-main);
}

.stat-txt {
  font-size: 0.64rem;
  color: var(--text-dim);
}

.stat-alive .stat-num {
  color: #10b981;
}

.stat-timeout .stat-num {
  color: #f43f5e;
}

.stat-ports .stat-num {
  color: #0284c7;
}

/* Tab Switch */
.tab-switch {
  display: flex;
  background-color: var(--bg-card-sub);
  border: 1px solid var(--border-color);
  border-radius: 6px;
  padding: 2px;
}

.tab-switch button {
  flex: 1;
  background: transparent;
  border: none;
  font-size: 0.72rem;
  color: var(--text-dim);
  padding: 5px 2px;
  border-radius: 4px;
  cursor: pointer;
  transition: all 0.15s;
}

.tab-switch button.active {
  background-color: var(--bg-card);
  color: #0284c7;
  font-weight: 600;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.08);
}

.import-panel {
  height: 215px;
  min-height: 215px;
  max-height: 215px;
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  gap: 10px;
  box-sizing: border-box;
}

.url-input-group {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.url-label {
  font-size: 0.74rem;
  color: var(--text-sub);
  font-weight: 500;
}

.url-tip-box {
  background-color: var(--bg-card-sub);
  border: 1px solid var(--border-color);
  border-radius: 6px;
  padding: 10px;
  font-size: 0.72rem;
  color: var(--text-dim);
  display: flex;
  gap: 6px;
  line-height: 1.5;
}

.tip-icon {
  font-size: 0.85rem;
}

.side-input {
  width: 100%;
  box-sizing: border-box;
  background-color: var(--bg-input);
  border: 1px solid var(--border-input);
  border-radius: 6px;
  padding: 6px 10px;
  font-size: 0.78rem;
  color: var(--text-main);
  outline: none;
  transition: border-color 0.2s;
}

.side-input:focus {
  border-color: #0284c7;
}

.side-textarea {
  flex: 1;
  min-height: 0;
  width: 100%;
  box-sizing: border-box;
  background-color: var(--bg-input);
  border: 1px solid var(--border-input);
  border-radius: 6px;
  padding: 6px 8px;
  font-size: 0.74rem;
  font-family: monospace;
  color: var(--text-main);
  outline: none;
  resize: none;
}

.side-textarea:focus {
  border-color: #0284c7;
}

/* Drag Drop Zone */
.drop-zone {
  height: 100%;
  box-sizing: border-box;
  border: 2px dashed var(--border-input);
  border-radius: 8px;
  padding: 10px;
  text-align: center;
  cursor: pointer;
  background-color: var(--bg-card-sub);
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 4px;
  transition: all 0.2s;
}

.drop-zone:hover,
.drop-zone.is-dragging {
  border-color: #0284c7;
  background-color: rgba(2, 132, 199, 0.06);
}

.drop-icon {
  font-size: 1.5rem;
}

.drop-text {
  margin: 0;
  font-size: 0.74rem;
  color: var(--text-main);
}

.drop-hint {
  font-size: 0.66rem;
  color: var(--text-dim);
}

/* Settings Card */
.settings-card {
  gap: 9px;
}

.setting-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 10px;
}

.setting-row label {
  font-size: 0.75rem;
  color: var(--text-sub);
  font-weight: 500;
  white-space: nowrap;
}

.setting-field-ctrl {
  width: 185px;
  height: 32px;
  box-sizing: border-box;
  background-color: var(--bg-input);
  border: 1px solid var(--border-input);
  border-radius: 6px;
  padding: 4px 8px;
  font-size: 0.76rem;
  color: var(--text-main);
  outline: none;
  transition: border-color 0.2s;
}

.setting-field-ctrl:focus {
  border-color: #0284c7;
}

.auto-filter-check {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 0.72rem;
  color: #0284c7;
  cursor: pointer;
  padding-top: 4px;
}

/* Sidebar Bottom (Power Button) */
.sidebar-bottom {
  margin-top: auto;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.btn-power-main {
  width: 100%;
  padding: 10px;
  border-radius: 8px;
  font-size: 0.88rem;
  font-weight: 700;
  border: none;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  transition: all 0.2s;
}

.power-stopped {
  background: linear-gradient(135deg, #10b981 0%, #059669 100%);
  color: #fff;
  box-shadow: 0 4px 14px rgba(16, 185, 129, 0.35);
}

.power-stopped:hover {
  filter: brightness(1.1);
  transform: translateY(-1px);
}

.power-running {
  background: linear-gradient(135deg, #ef4444 0%, #dc2626 100%);
  color: #fff;
  box-shadow: 0 4px 14px rgba(239, 68, 68, 0.35);
}

.power-running:hover {
  filter: brightness(1.1);
}

.power-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background-color: #fff;
}

.persistence-hint {
  font-size: 0.66rem;
  color: var(--text-dim);
  text-align: center;
}

/* ==================== RIGHT MAIN CONTENT ==================== */
.main-content {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  height: 100vh;
  overflow: hidden;
}

/* Header (2-tier layout) */
.main-header {
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  background-color: var(--bg-sidebar);
  border-bottom: 1px solid var(--border-color);
}

.toolbar-top {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 18px 8px 18px;
  gap: 12px;
}

.toolbar-search {
  position: relative;
  flex: 1;
  max-width: 340px;
  min-width: 200px;
}

.search-icon {
  position: absolute;
  left: 9px;
  top: 50%;
  transform: translateY(-50%);
  width: 14px;
  height: 14px;
  color: var(--text-dim);
  pointer-events: none;
}

.search-input {
  width: 100%;
  box-sizing: border-box;
  background-color: var(--bg-card);
  border: 1px solid var(--border-input);
  border-radius: 6px;
  padding: 6px 28px 6px 30px;
  font-size: 0.78rem;
  color: var(--text-main);
  outline: none;
  height: 32px;
  transition: border-color 0.2s;
}

.search-input:focus {
  border-color: #0284c7;
}

.search-clear-btn {
  position: absolute;
  right: 8px;
  top: 50%;
  transform: translateY(-50%);
  background: none;
  border: none;
  font-size: 0.75rem;
  color: var(--text-dim);
  cursor: pointer;
  padding: 2px 4px;
  border-radius: 50%;
}

.search-clear-btn:hover {
  color: var(--text-main);
}

.action-btns {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
  white-space: nowrap;
}

.toolbar-bottom {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 6px 18px 10px 18px;
  border-top: 1px solid rgba(128, 128, 128, 0.08);
  gap: 10px;
  overflow-x: auto;
}

.toolbar-bottom::-webkit-scrollbar {
  height: 3px;
}
.toolbar-bottom::-webkit-scrollbar-thumb {
  background: var(--scroll-thumb);
  border-radius: 2px;
}

.filter-section {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: nowrap;
}

.filter-group {
  display: flex;
  align-items: center;
  gap: 4px;
  white-space: nowrap;
}

.filter-label {
  font-size: 0.72rem;
  color: var(--text-dim);
  font-weight: 600;
  margin-right: 2px;
}

.filter-divider {
  width: 1px;
  height: 14px;
  background-color: var(--border-color);
  margin: 0 4px;
}

.pill-tag {
  background-color: var(--bg-card);
  border: 1px solid var(--border-input);
  color: var(--text-sub);
  font-size: 0.72rem;
  padding: 3px 8px;
  border-radius: 5px;
  cursor: pointer;
  white-space: nowrap;
  transition: all 0.15s;
  line-height: 1.2;
}

.pill-tag:hover {
  border-color: #0284c7;
  color: #0284c7;
}

.pill-tag.active {
  background-color: rgba(2, 132, 199, 0.12);
  border-color: #0284c7;
  color: #0284c7;
  font-weight: 600;
}

.pill-green.active {
  background-color: rgba(16, 185, 129, 0.12);
  border-color: #10b981;
  color: #10b981;
}

.pill-red.active {
  background-color: rgba(244, 63, 94, 0.12);
  border-color: #f43f5e;
  color: #f43f5e;
}

.node-count-badge {
  font-size: 0.72rem;
  color: var(--text-dim);
  white-space: nowrap;
  flex-shrink: 0;
}

.node-count-badge strong {
  color: var(--text-main);
}

/* Buttons */
.btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 5px;
  height: 32px;
  padding: 0 12px;
  border-radius: 6px;
  font-size: 0.76rem;
  font-weight: 600;
  cursor: pointer;
  border: none;
  white-space: nowrap;
  flex-shrink: 0;
  transition: all 0.15s;
}

.btn-primary {
  background-color: #0284c7;
  color: #fff;
}

.btn-primary:hover {
  background-color: #0369a1;
}

.btn-test {
  background: linear-gradient(135deg, #0284c7 0%, #2563eb 100%);
  color: #fff;
  box-shadow: 0 2px 8px rgba(37, 99, 235, 0.25);
}

.btn-test:hover {
  filter: brightness(1.08);
  transform: translateY(-0.5px);
}

.btn-warn {
  background-color: rgba(245, 158, 11, 0.15);
  color: #f59e0b;
  border: 1px solid rgba(245, 158, 11, 0.35);
}

.btn-warn:hover {
  background-color: rgba(245, 158, 11, 0.25);
}

.btn-danger {
  background-color: rgba(244, 63, 94, 0.15);
  color: #f43f5e;
  border: 1px solid rgba(244, 63, 94, 0.35);
}

.btn-danger:hover {
  background-color: rgba(244, 63, 94, 0.25);
}

.btn-accent {
  background-color: #0f766e;
  color: #fff;
}

.btn-accent:hover {
  background-color: #115e59;
}

.btn-secondary {
  background-color: var(--btn-sec-bg);
  color: var(--btn-sec-txt);
}

.btn-block {
  width: 100%;
  justify-content: center;
}

.btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
  transform: none;
}

/* ==================== TABLE SCROLL AREA ==================== */
.table-area {
  flex: 1;
  min-height: 0;
  margin: 12px 18px 14px 18px;
  background-color: var(--bg-card);
  border: 1px solid var(--border-color);
  border-radius: 8px;
  overflow-y: auto; /* INTERNAL SCROLLBAR ONLY */
  overflow-x: hidden;
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.05);
}

/* Slim Custom Scrollbar */
.table-area::-webkit-scrollbar {
  width: 5px;
}
.table-area::-webkit-scrollbar-track {
  background: var(--scroll-track);
}
.table-area::-webkit-scrollbar-thumb {
  background: var(--scroll-thumb);
  border-radius: 3px;
}
.table-area::-webkit-scrollbar-thumb:hover {
  background: #0284c7;
}

.data-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 0.78rem;
}

.data-table thead {
  position: sticky;
  top: 0;
  background-color: var(--bg-table-head);
  color: var(--text-sub);
  z-index: 10;
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.05);
}

.data-table th {
  padding: 8px 10px;
  border-bottom: 1px solid var(--border-color);
  text-align: left;
  vertical-align: middle;
  white-space: nowrap;
  font-weight: 600;
  user-select: none;
}

.data-table td {
  padding: 8px 10px;
  border-bottom: 1px solid var(--border-color);
  text-align: left;
  vertical-align: middle;
  white-space: nowrap;
}

.data-table tbody tr:hover {
  background-color: var(--bg-table-hover);
}

.data-table tbody tr.row-active {
  background-color: var(--bg-table-active);
}

.table-empty {
  text-align: center;
  padding: 70px 0;
  color: var(--text-dim);
}

.empty-icon {
  font-size: 2.2rem;
  margin-bottom: 6px;
}

.port-tag {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  padding: 2px 7px;
  border-radius: 4px;
  font-family: ui-monospace, SFMono-Regular, "Cascadia Code", "SF Mono", Consolas, monospace;
  font-size: 0.74rem;
  font-weight: 700;
  background: rgba(2, 132, 199, 0.12);
  color: #0284c7;
  border: 1px solid rgba(2, 132, 199, 0.3);
  white-space: nowrap;
}

.port-tag.clickable:hover {
  background: rgba(2, 132, 199, 0.22);
  cursor: pointer;
}

.node-cell {
  max-width: 280px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  font-weight: 500;
  color: var(--text-main);
}

.proto-tag {
  display: inline-block;
  padding: 2px 6px;
  border-radius: 4px;
  font-size: 0.66rem;
  font-weight: 700;
  white-space: nowrap;
}

.proto-vless {
  background-color: rgba(168, 85, 247, 0.15);
  color: #9333ea;
}
.proto-vmess {
  background-color: rgba(236, 72, 153, 0.15);
  color: #db2777;
}
.proto-ss {
  background-color: rgba(59, 130, 246, 0.15);
  color: #2563eb;
}
.proto-trojan {
  background-color: rgba(234, 179, 8, 0.15);
  color: #ca8a04;
}
.proto-hysteria2 {
  background-color: rgba(20, 184, 166, 0.15);
  color: #0d9488;
}

.delay-tag {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 4px;
  padding: 2px 8px;
  border-radius: 12px;
  font-size: 0.7rem;
  font-weight: 600;
  white-space: nowrap;
}

.delay-green {
  background-color: rgba(16, 185, 129, 0.15);
  color: #10b981;
}
.delay-yellow {
  background-color: rgba(234, 179, 8, 0.15);
  color: #d97706;
}
.delay-orange {
  background-color: rgba(249, 115, 22, 0.15);
  color: #ea580c;
}
.delay-red {
  background-color: rgba(244, 63, 94, 0.15);
  color: #f43f5e;
}
.delay-testing {
  background-color: rgba(100, 116, 139, 0.15);
  color: var(--text-dim);
}

.dim-text {
  color: var(--text-dim);
}

.font-mono {
  font-family: ui-monospace, SFMono-Regular, "Cascadia Code", "SF Mono", Consolas, "Liberation Mono", Menlo, monospace;
  font-size: 0.74rem;
}

.action-cell {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 4px;
  white-space: nowrap;
}

.btn-row {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 2px;
  height: 25px;
  padding: 0 7px;
  background-color: var(--bg-input);
  border: 1px solid var(--border-input);
  color: var(--text-sub);
  border-radius: 4px;
  font-size: 0.7rem;
  cursor: pointer;
  white-space: nowrap;
  flex-shrink: 0;
  line-height: 1;
  transition: all 0.15s;
}

.btn-row:hover {
  border-color: #0284c7;
  color: #0284c7;
}

.btn-row-speed:hover {
  border-color: #f59e0b;
  color: #f59e0b;
}

.btn-row-del:hover {
  border-color: #f43f5e;
  color: #f43f5e;
}

/* ==================== EXPORT MODAL ==================== */
.modal-backdrop {
  position: fixed;
  inset: 0;
  background-color: rgba(0, 0, 0, 0.6);
  backdrop-filter: blur(2px);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 999;
}

.modal-card {
  width: 90%;
  max-width: 620px;
  background-color: var(--bg-card);
  border: 1px solid var(--border-color);
  border-radius: 12px;
  overflow: hidden;
  box-shadow: 0 16px 36px rgba(0, 0, 0, 0.3);
}

.modal-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 14px 18px;
  border-bottom: 1px solid var(--border-color);
}

.modal-header h3 {
  margin: 0;
  font-size: 0.95rem;
  color: var(--text-main);
}

.modal-close {
  background: transparent;
  border: none;
  color: var(--text-dim);
  font-size: 1.1rem;
  cursor: pointer;
}

.modal-body {
  padding: 16px 18px;
}

.radio-group {
  display: flex;
  gap: 14px;
  margin-bottom: 10px;
}

.radio-label {
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: 0.76rem;
  color: var(--text-sub);
  cursor: pointer;
}

.export-box {
  width: 100%;
  box-sizing: border-box;
  background-color: var(--bg-input);
  border: 1px solid var(--border-input);
  border-radius: 6px;
  padding: 10px;
  font-family: monospace;
  font-size: 0.78rem;
  color: #0284c7;
  outline: none;
  resize: none;
}

.modal-footer {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  padding: 12px 18px;
  background-color: var(--bg-card-sub);
  border-top: 1px solid var(--border-color);
}

/* Spinners */
.spinner {
  width: 12px;
  height: 12px;
  border: 2px solid rgba(255, 255, 255, 0.3);
  border-top-color: #fff;
  border-radius: 50%;
  animation: spin 0.7s linear infinite;
}

.mini-spinner {
  display: inline-block;
  width: 8px;
  height: 8px;
  border: 1.5px solid rgba(255, 255, 255, 0.3);
  border-top-color: var(--text-dim);
  border-radius: 50%;
  animation: spin 0.7s linear infinite;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

/* Toast */
.toast-popup {
  position: fixed;
  bottom: 24px;
  left: 50%;
  transform: translateX(-50%);
  background-color: var(--bg-sidebar);
  border: 1px solid #0284c7;
  color: var(--text-main);
  padding: 8px 20px;
  border-radius: 20px;
  font-size: 0.82rem;
  box-shadow: 0 8px 20px rgba(0, 0, 0, 0.25);
  z-index: 1000;
}

.toast-fade-enter-active,
.toast-fade-leave-active {
  transition: all 0.25s ease;
}

.toast-fade-enter-from,
.toast-fade-leave-to {
  opacity: 0;
  transform: translate(-50%, 10px);
}

/* Global Drag & Drop Overlay */
.global-drag-overlay {
  position: fixed;
  inset: 0;
  background-color: rgba(9, 13, 22, 0.85);
  backdrop-filter: blur(6px);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 9999;
  pointer-events: none;
}

.drag-box {
  border: 3px dashed #0284c7;
  background-color: var(--bg-card);
  border-radius: 16px;
  padding: 40px 60px;
  text-align: center;
  box-shadow: 0 16px 40px rgba(2, 132, 199, 0.3);
}

.drag-icon-large {
  font-size: 3.5rem;
  margin-bottom: 12px;
}

.drag-box h2 {
  margin: 0 0 8px 0;
  font-size: 1.3rem;
  color: var(--text-main);
}

.drag-box p {
  margin: 0;
  font-size: 0.85rem;
  color: var(--text-sub);
}

.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.2s ease;
}

.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}
</style>