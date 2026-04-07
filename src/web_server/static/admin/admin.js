const state = {
  sites: [],
  selectedSiteId: null,
  activeLogTab: "parse",
  autoRefresh: true,
  autoRefreshTimer: null,
  runtime: null,
  logs: null,
};

const dom = {
  siteList: document.getElementById("site-list"),
  listSummary: document.getElementById("list-summary"),
  detailGrid: document.getElementById("detail-grid"),
  runtimeGrid: document.getElementById("runtime-grid"),
  runtimeFlags: document.getElementById("runtime-flags"),
  runtimeMessage: document.getElementById("runtime-message"),
  selectionBanner: document.getElementById("selection-banner"),
  statusStrip: document.getElementById("status-strip"),
  statusStripBadges: document.getElementById("status-strip-badges"),
  statusStripSummary: document.getElementById("status-strip-summary"),
  statusProgress: document.getElementById("status-progress"),
  statusProgressBar: document.getElementById("status-progress-bar"),
  statusProgressText: document.getElementById("status-progress-text"),
  logTabs: document.getElementById("log-tabs"),
  logSummary: document.getElementById("log-summary"),
  logContent: document.getElementById("log-content"),
  toastRegion: document.getElementById("toast-region"),
  form: document.getElementById("site-form"),
  formTitle: document.getElementById("form-title"),
  projectName: document.getElementById("project-name"),
  projectCode: document.getElementById("project-code"),
  projectPath: document.getElementById("project-path"),
  manualDbNums: document.getElementById("manual-db-nums"),
  bindHost: document.getElementById("bind-host"),
  dbPort: document.getElementById("db-port"),
  webPort: document.getElementById("web-port"),
  dbUser: document.getElementById("db-user"),
  dbPassword: document.getElementById("db-password"),
  createSiteBtn: document.getElementById("create-site-btn"),
  resetFormBtn: document.getElementById("reset-form-btn"),
  cancelEditBtn: document.getElementById("cancel-edit-btn"),
  refreshAllBtn: document.getElementById("refresh-all-btn"),
  autoRefreshToggle: document.getElementById("auto-refresh-toggle"),
  searchInput: document.getElementById("site-search"),
  parseBtn: document.getElementById("parse-btn"),
  startBtn: document.getElementById("start-btn"),
  stopBtn: document.getElementById("stop-btn"),
  deleteBtn: document.getElementById("delete-btn"),
  statTotal: document.getElementById("stat-total-sites"),
  statRunning: document.getElementById("stat-running-sites"),
  statFailed: document.getElementById("stat-failed-sites"),
};

function normalizeEnvelope(payload) {
  if (payload && typeof payload === "object" && "success" in payload) {
    return payload;
  }
  return { success: true, message: "", data: payload };
}

async function request(url, init) {
  const response = await fetch(url, {
    headers: {
      "Content-Type": "application/json",
      ...(init && init.headers ? init.headers : {}),
    },
    ...init,
  });

  const text = await response.text();
  let payload = null;
  if (text) {
    try {
      payload = JSON.parse(text);
    } catch (_error) {
      payload = { success: response.ok, message: text, data: null };
    }
  }

  const envelope = normalizeEnvelope(payload);
  if (!response.ok || envelope.success === false) {
    throw new Error(envelope.message || text || `HTTP ${response.status}`);
  }
  return envelope.data;
}

function showToast(message, type = "info") {
  const toast = document.createElement("div");
  toast.className = `toast ${type}`;
  toast.textContent = message;
  dom.toastRegion.appendChild(toast);
  window.setTimeout(() => toast.remove(), 3200);
}

function formatValue(value) {
  if (value === null || value === undefined || value === "") {
    return "—";
  }
  if (Array.isArray(value)) {
    return value.length ? value.join(", ") : "—";
  }
  if (typeof value === "boolean") {
    return value ? "是" : "否";
  }
  return String(value);
}

function formatDateTime(value) {
  if (!value) {
    return "—";
  }
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return String(value);
  }
  return date.toLocaleString("zh-CN", {
    hour12: false,
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

function parseManualDbNums(value) {
  if (!value.trim()) {
    return [];
  }
  const values = value
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean)
    .map((item) => Number(item));

  if (!values.length || values.some((item) => !Number.isInteger(item) || item <= 0)) {
    throw new Error("限制 dbnum 只接受正整数，多个值请用逗号分隔");
  }

  return [...new Set(values)].sort((left, right) => left - right);
}

function escapeHtml(value) {
  return String(value)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/\"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function badgeClass(prefix, value) {
  const safe = String(value || "default").toLowerCase();
  return `badge ${prefix}-${safe}`;
}

function selectedSite() {
  return state.sites.find((item) => item.site_id === state.selectedSiteId) || null;
}

function filteredSites() {
  const keyword = dom.searchInput.value.trim().toLowerCase();
  if (!keyword) {
    return state.sites;
  }
  return state.sites.filter((site) => {
    return [
      site.project_name,
      site.site_id,
      site.project_path,
      site.db_port,
      site.web_port,
    ]
      .map((value) => String(value || "").toLowerCase())
      .some((value) => value.includes(keyword));
  });
}

function normalizeStreams(logs) {
  if (!logs || !Array.isArray(logs.streams) || !logs.streams.length) {
    return [
      {
        key: "parse",
        label: "解析日志",
        path: "—",
        exists: Array.isArray(logs?.parse_log) && logs.parse_log.length > 0,
        updated_at: null,
        line_count: Array.isArray(logs?.parse_log) ? logs.parse_log.length : 0,
        last_line: Array.isArray(logs?.parse_log) && logs.parse_log.length ? logs.parse_log[logs.parse_log.length - 1] : null,
      },
      {
        key: "db",
        label: "数据库日志",
        path: "—",
        exists: Array.isArray(logs?.db_log) && logs.db_log.length > 0,
        updated_at: null,
        line_count: Array.isArray(logs?.db_log) ? logs.db_log.length : 0,
        last_line: Array.isArray(logs?.db_log) && logs.db_log.length ? logs.db_log[logs.db_log.length - 1] : null,
      },
      {
        key: "web",
        label: "站点日志",
        path: "—",
        exists: Array.isArray(logs?.web_log) && logs.web_log.length > 0,
        updated_at: null,
        line_count: Array.isArray(logs?.web_log) ? logs.web_log.length : 0,
        last_line: Array.isArray(logs?.web_log) && logs.web_log.length ? logs.web_log[logs.web_log.length - 1] : null,
      },
    ];
  }
  return logs.streams;
}

function streamMap(logs) {
  return {
    parse: Array.isArray(logs?.parse_log) ? logs.parse_log : [],
    db: Array.isArray(logs?.db_log) ? logs.db_log : [],
    web: Array.isArray(logs?.web_log) ? logs.web_log : [],
  };
}

function currentLogStream(logs) {
  return normalizeStreams(logs).find((item) => item.key === state.activeLogTab) || normalizeStreams(logs)[0] || null;
}

function isBusyRuntime(runtime) {
  if (!runtime) {
    return false;
  }
  return Boolean(
    runtime.parse_running
      || runtime.parse_status === "Running"
      || runtime.status === "Starting"
      || runtime.status === "Stopping",
  );
}

function pollIntervalMs() {
  if (!state.autoRefresh) {
    return 0;
  }
  if (isBusyRuntime(state.runtime)) {
    return 2000;
  }
  if (state.selectedSiteId) {
    return 5000;
  }
  return 8000;
}

function progressPercent(runtime) {
  if (!runtime) {
    return 0;
  }
  if (runtime.status === "Running") {
    return 100;
  }
  if (runtime.status === "Starting") {
    return 82;
  }
  if (runtime.status === "Stopping") {
    return 35;
  }
  if (runtime.parse_running || runtime.parse_status === "Running") {
    return 58;
  }
  if (runtime.parse_status === "Parsed") {
    return 72;
  }
  if (runtime.parse_status === "Failed" || runtime.status === "Failed") {
    return 100;
  }
  return 12;
}

function renderStats() {
  const running = state.sites.filter((site) => site.status === "Running").length;
  const failed = state.sites.filter((site) => site.status === "Failed").length;
  dom.statTotal.textContent = String(state.sites.length);
  dom.statRunning.textContent = String(running);
  dom.statFailed.textContent = String(failed);
}

function renderSiteList() {
  const items = filteredSites();
  renderStats();

  if (!items.length) {
    dom.listSummary.textContent = state.sites.length
      ? "没有匹配的站点。"
      : "当前还没有站点，请先创建。";
    dom.siteList.innerHTML = '<div class="empty-card">暂无可显示的站点</div>';
    return;
  }

  dom.listSummary.textContent = `共 ${state.sites.length} 个站点，当前显示 ${items.length} 个。`;
  dom.siteList.innerHTML = items
    .map((site) => {
      const active = site.site_id === state.selectedSiteId ? "is-active" : "";
      const entry = site.entry_url
        ? `<a href="${escapeHtml(site.entry_url)}" target="_blank" rel="noreferrer">打开站点</a>`
        : "未生成入口";
      const errorText = site.last_error
        ? `<p class="site-path">${escapeHtml(site.last_error)}</p>`
        : "";
      const manualDbNums = Array.isArray(site.manual_db_nums) && site.manual_db_nums.length
        ? `<span class="badge status-default">DBNUM ${escapeHtml(site.manual_db_nums.join(","))}</span>`
        : "";
      return `
        <button class="site-item ${active}" type="button" data-site-id="${escapeHtml(site.site_id)}" role="listitem">
          <div class="site-item-header">
            <div>
              <p class="site-name">${escapeHtml(site.project_name)}</p>
              <span class="site-id">${escapeHtml(site.site_id)}</span>
            </div>
            <span class="${badgeClass("status", site.status)}">${escapeHtml(site.status)}</span>
          </div>
          <p class="site-path">${escapeHtml(site.project_path)}</p>
          ${errorText}
          <div class="site-item-meta">
            <div class="meta-group">
              <span class="${badgeClass("parse", site.parse_status)}">解析 ${escapeHtml(site.parse_status)}</span>
              <span class="badge status-default">DB ${escapeHtml(site.db_port)}</span>
              <span class="badge status-default">WEB ${escapeHtml(site.web_port)}</span>
              ${manualDbNums}
            </div>
          </div>
          <div class="site-item-footer">
            <span class="site-id">${escapeHtml(site.updated_at || "未更新时间")}</span>
            <span class="site-id">${entry}</span>
          </div>
        </button>
      `;
    })
    .join("");
}

function renderDetail(site) {
  if (!site) {
    dom.selectionBanner.className = "selection-banner empty";
    dom.selectionBanner.textContent = "未选中站点。请从左侧列表选择，或直接新建。";
    dom.detailGrid.className = "kv-grid empty-state";
    dom.detailGrid.textContent = "暂无详情";
    return;
  }

  dom.selectionBanner.className = "selection-banner";
  dom.selectionBanner.textContent = `${site.project_name} · ${site.site_id}`;
  const entries = [
    ["项目名", site.project_name],
    ["项目代号", site.project_code],
    ["项目路径", site.project_path],
    ["限制 dbnum", site.manual_db_nums],
    ["配置文件", site.config_path],
    ["运行目录", site.runtime_dir],
    ["数据目录", site.db_data_path],
    ["绑定地址", site.bind_host],
    ["数据库端口", site.db_port],
    ["站点端口", site.web_port],
    ["当前状态", site.status],
    ["解析状态", site.parse_status],
    [
      "访问入口",
      site.entry_url
        ? `<a href="${escapeHtml(site.entry_url)}" target="_blank" rel="noreferrer">${escapeHtml(site.entry_url)}</a>`
        : "—",
    ],
    ["创建时间", formatDateTime(site.created_at)],
    ["更新时间", formatDateTime(site.updated_at)],
  ];

  dom.detailGrid.className = "kv-grid";
  dom.detailGrid.innerHTML = entries
    .map(([label, value]) => {
      const htmlValue = label === "访问入口" ? value : escapeHtml(formatValue(value));
      return `<dl class="kv-item"><dt>${escapeHtml(label)}</dt><dd>${htmlValue}</dd></dl>`;
    })
    .join("");
}

function renderStatusStrip(site, runtime) {
  if (!site || !runtime) {
    dom.statusStrip.className = "status-strip empty";
    dom.statusStripBadges.innerHTML = "";
    dom.statusStripSummary.className = "status-strip-summary empty-state";
    dom.statusStripSummary.textContent = "选择站点后显示解析进度、轮询频率和最新状态。";
    dom.statusProgress.classList.add("hidden");
    dom.statusProgress.setAttribute("aria-hidden", "true");
    dom.statusProgressText.classList.add("hidden");
    dom.statusProgressText.textContent = "";
    dom.statusProgressBar.style.width = "0%";
    return;
  }

  const recentSummary = runtime.recent_activity?.summary || runtime.last_error || "暂无最新活动";
  const lastSeen = runtime.last_log_at ? `最近日志 ${formatDateTime(runtime.last_log_at)}` : "尚无日志时间";
  const intervalLabel = `${Math.round(pollIntervalMs() / 1000)} 秒轮询`;

  dom.statusStrip.className = "status-strip";
  dom.statusStripBadges.innerHTML = [
    `<span class="${badgeClass("status", runtime.status)}">${escapeHtml(runtime.current_stage_label || runtime.status)}</span>`,
    `<span class="${badgeClass("parse", runtime.parse_status)}">解析 ${escapeHtml(runtime.parse_status)}</span>`,
    `<span class="${badgeClass("bool", runtime.parse_running)}">解析进程${runtime.parse_running ? "在线" : "离线"}</span>`,
  ].join("");
  dom.statusStripSummary.className = "status-strip-summary";
  dom.statusStripSummary.innerHTML = `
    <strong>${escapeHtml(site.project_name)}</strong>
    <span>${escapeHtml(recentSummary)}</span>
    <span>${escapeHtml(lastSeen)} · ${escapeHtml(intervalLabel)}</span>
  `;

  const busy = isBusyRuntime(runtime);
  dom.statusProgress.classList.toggle("hidden", !busy);
  dom.statusProgress.setAttribute("aria-hidden", busy ? "false" : "true");
  dom.statusProgressText.classList.toggle("hidden", !busy);
  dom.statusProgressText.textContent = busy
    ? `${runtime.current_stage_label || runtime.status} · ${runtime.recent_activity?.label || "等待新日志"}`
    : `${runtime.current_stage_label || runtime.status} · 当前无需高频轮询`;
  dom.statusProgressBar.style.width = `${progressPercent(runtime)}%`;
}

function renderRuntime(runtime) {
  if (!runtime) {
    dom.runtimeGrid.className = "runtime-grid empty-state";
    dom.runtimeGrid.textContent = "暂无运行态";
    dom.runtimeFlags.innerHTML = "";
    dom.runtimeMessage.classList.add("hidden");
    dom.runtimeMessage.textContent = "";
    return;
  }

  const cards = [
    ["当前阶段", runtime.current_stage_label || runtime.status],
    ["站点状态", runtime.status],
    ["解析状态", runtime.parse_status],
    ["当前活跃日志", runtime.recent_activity?.label || runtime.active_log_kind || "—"],
    ["最后日志时间", formatDateTime(runtime.last_log_at)],
    ["最近活动", runtime.recent_activity?.summary || "—"],
    ["数据库 PID", runtime.db_pid],
    ["站点 PID", runtime.web_pid],
    ["解析 PID", runtime.parse_pid],
    ["数据库端口", runtime.db_port],
    ["站点端口", runtime.web_port],
    ["入口地址", runtime.entry_url || "—"],
  ];

  dom.runtimeGrid.className = "runtime-grid";
  dom.runtimeGrid.innerHTML = cards
    .map(
      ([label, value]) =>
        `<div class="runtime-card"><span>${escapeHtml(label)}</span><strong>${escapeHtml(formatValue(value))}</strong></div>`,
    )
    .join("");

  dom.runtimeFlags.innerHTML = [
    `<span class="${badgeClass("bool", runtime.db_running)}">数据库${runtime.db_running ? "已运行" : "未运行"}</span>`,
    `<span class="${badgeClass("bool", runtime.web_running)}">站点${runtime.web_running ? "已运行" : "未运行"}</span>`,
    `<span class="${badgeClass("bool", runtime.parse_running)}">解析${runtime.parse_running ? "进行中" : "已结束"}</span>`,
  ].join("");

  if (runtime.last_error) {
    dom.runtimeMessage.classList.remove("hidden");
    dom.runtimeMessage.textContent = runtime.last_error;
  } else {
    dom.runtimeMessage.classList.add("hidden");
    dom.runtimeMessage.textContent = "";
  }
}

function renderLogTabs(logs) {
  const streams = normalizeStreams(logs);
  if (!streams.some((item) => item.key === state.activeLogTab)) {
    state.activeLogTab = streams[0]?.key || "parse";
  }

  dom.logTabs.innerHTML = streams
    .map((stream) => {
      const active = stream.key === state.activeLogTab;
      const updated = stream.updated_at ? formatDateTime(stream.updated_at) : "未更新";
      return `
        <button
          class="log-tab ${active ? "is-active" : ""}"
          data-log-tab="${escapeHtml(stream.key)}"
          type="button"
          role="tab"
          aria-selected="${active ? "true" : "false"}"
        >
          <span>${escapeHtml(stream.label)}</span>
          <small>${escapeHtml(updated)}</small>
        </button>
      `;
    })
    .join("");
}

function renderLogs(logs) {
  if (!logs) {
    renderLogTabs(null);
    dom.logSummary.className = "log-summary empty-state";
    dom.logSummary.textContent = "请选择站点后查看日志状态。";
    dom.logContent.textContent = "请选择站点后查看日志。";
    return;
  }

  renderLogTabs(logs);
  const lines = streamMap(logs)[state.activeLogTab] || [];
  const stream = currentLogStream(logs);
  if (!stream) {
    dom.logSummary.className = "log-summary empty-state";
    dom.logSummary.textContent = "当前没有可展示的日志。";
    dom.logContent.textContent = "当前日志为空。";
    return;
  }

  dom.logSummary.className = "log-summary";
  dom.logSummary.innerHTML = `
    <div class="log-summary-card">
      <span>日志文件</span>
      <strong>${escapeHtml(formatValue(stream.path))}</strong>
    </div>
    <div class="log-summary-card">
      <span>最后更新</span>
      <strong>${escapeHtml(formatDateTime(stream.updated_at))}</strong>
    </div>
    <div class="log-summary-card">
      <span>内容状态</span>
      <strong>${escapeHtml(stream.exists ? `已写入 ${stream.line_count} 行` : "尚未生成")}</strong>
    </div>
    <div class="log-summary-card">
      <span>最后一条</span>
      <strong>${escapeHtml(formatValue(stream.last_line))}</strong>
    </div>
  `;
  dom.logContent.textContent = lines.length ? lines.join("\n") : "当前日志为空。";
}

function fillForm(site) {
  if (!site) {
    dom.formTitle.textContent = "新建站点";
    dom.form.reset();
    dom.dbPassword.value = "";
    return;
  }

  dom.formTitle.textContent = `编辑站点 · ${site.project_name}`;
  dom.projectName.value = site.project_name || "";
  dom.projectCode.value = site.project_code || "";
  dom.projectPath.value = site.project_path || "";
  dom.manualDbNums.value = Array.isArray(site.manual_db_nums) ? site.manual_db_nums.join(",") : "";
  dom.bindHost.value = site.bind_host || "";
  dom.dbPort.value = site.db_port || "";
  dom.webPort.value = site.web_port || "";
  dom.dbUser.value = "";
  dom.dbPassword.value = "";
}

function clearPanels() {
  state.runtime = null;
  state.logs = null;
  renderDetail(selectedSite());
  renderStatusStrip(null, null);
  renderRuntime(null);
  renderLogs(null);
  setupAutoRefresh();
}

function setSelectedSite(siteId) {
  state.selectedSiteId = siteId;
  const site = selectedSite();
  fillForm(site);
  renderSiteList();
  renderDetail(site);
  renderStatusStrip(site, state.runtime);
  setupAutoRefresh();
}

async function loadSites(options = {}) {
  const { keepSelection = true } = options;
  const sites = await request("/api/admin/sites");
  state.sites = Array.isArray(sites) ? sites : [];

  if (!keepSelection) {
    state.selectedSiteId = null;
  }
  if (state.selectedSiteId && !selectedSite()) {
    state.selectedSiteId = null;
  }

  renderSiteList();
  renderDetail(selectedSite());
  fillForm(selectedSite());

  if (state.selectedSiteId) {
    await refreshSelectedPanels({ silent: true, skipList: true });
  } else {
    clearPanels();
  }
  setupAutoRefresh();
}

async function loadSiteDetail(siteId) {
  const site = await request(`/api/admin/sites/${encodeURIComponent(siteId)}`);
  const index = state.sites.findIndex((item) => item.site_id === site.site_id);
  if (index >= 0) {
    state.sites.splice(index, 1, site);
  } else {
    state.sites.unshift(site);
  }
  renderSiteList();
  renderDetail(site);
  fillForm(site);
  return site;
}

async function refreshSelectedPanels(options = {}) {
  const { silent = false, skipList = false } = options;
  const siteId = state.selectedSiteId;
  if (!siteId) {
    clearPanels();
    return;
  }

  try {
    const sitePromise = skipList ? request(`/api/admin/sites/${encodeURIComponent(siteId)}`) : loadSiteDetail(siteId);
    const [site, runtime, logs] = await Promise.all([
      sitePromise,
      request(`/api/admin/sites/${encodeURIComponent(siteId)}/runtime`),
      request(`/api/admin/sites/${encodeURIComponent(siteId)}/logs`),
    ]);
    if (skipList) {
      const index = state.sites.findIndex((item) => item.site_id === site.site_id);
      if (index >= 0) {
        state.sites.splice(index, 1, site);
      } else {
        state.sites.unshift(site);
      }
      renderSiteList();
      renderDetail(site);
      fillForm(site);
    }
    state.runtime = runtime;
    state.logs = logs;
    renderStatusStrip(site, runtime);
    renderRuntime(runtime);
    renderLogs(logs);
    setupAutoRefresh();
  } catch (error) {
    if (!silent) {
      showToast(error.message, "error");
    }
  }
}

function collectFormPayload() {
  const manualDbNums = parseManualDbNums(dom.manualDbNums.value);
  return {
    project_name: dom.projectName.value.trim(),
    project_code: Number(dom.projectCode.value),
    project_path: dom.projectPath.value.trim(),
    manual_db_nums: manualDbNums,
    bind_host: dom.bindHost.value.trim() || null,
    db_port: Number(dom.dbPort.value),
    web_port: Number(dom.webPort.value),
    db_user: dom.dbUser.value.trim() || null,
    db_password: dom.dbPassword.value || null,
  };
}

function collectUpdatePayload() {
  const payload = collectFormPayload();
  return {
    project_name: payload.project_name,
    project_code: payload.project_code,
    project_path: payload.project_path,
    manual_db_nums: payload.manual_db_nums,
    bind_host: payload.bind_host,
    db_port: payload.db_port,
    web_port: payload.web_port,
    db_user: payload.db_user,
    db_password: payload.db_password,
  };
}

async function handleSubmit(event) {
  event.preventDefault();

  try {
    const isEdit = Boolean(state.selectedSiteId);
    const payload = isEdit ? collectUpdatePayload() : collectFormPayload();
    const method = isEdit ? "PUT" : "POST";
    const url = isEdit
      ? `/api/admin/sites/${encodeURIComponent(state.selectedSiteId)}`
      : "/api/admin/sites";
    const site = await request(url, {
      method,
      body: JSON.stringify(payload),
    });
    showToast(isEdit ? "站点已更新" : "站点已创建", "success");
    state.selectedSiteId = site.site_id;
    await loadSites();
  } catch (error) {
    showToast(error.message, "error");
  }
}

async function triggerAction(action) {
  const site = selectedSite();
  if (!site) {
    showToast("请先选择站点", "info");
    return;
  }

  try {
    await request(`/api/admin/sites/${encodeURIComponent(site.site_id)}/${action}`, {
      method: "POST",
    });
    const label = action === "parse" ? "解析" : action === "start" ? "启动" : "停止";
    showToast(`${label}请求已提交`, "success");
    await refreshSelectedPanels({ silent: true, skipList: true });
    setupAutoRefresh();
  } catch (error) {
    showToast(error.message, "error");
  }
}

async function handleDelete() {
  const site = selectedSite();
  if (!site) {
    showToast("请先选择站点", "info");
    return;
  }
  const confirmed = window.confirm(`确认删除站点 “${site.project_name}” 吗？`);
  if (!confirmed) {
    return;
  }

  try {
    await request(`/api/admin/sites/${encodeURIComponent(site.site_id)}`, {
      method: "DELETE",
    });
    showToast("站点已删除", "success");
    state.selectedSiteId = null;
    await loadSites({ keepSelection: false });
  } catch (error) {
    showToast(error.message, "error");
  }
}

function switchToCreateMode() {
  state.selectedSiteId = null;
  renderSiteList();
  fillForm(null);
  clearPanels();
}

function bindEvents() {
  dom.siteList.addEventListener("click", (event) => {
    const button = event.target.closest("[data-site-id]");
    if (!button) {
      return;
    }
    setSelectedSite(button.getAttribute("data-site-id"));
    refreshSelectedPanels({ silent: true, skipList: true });
  });

  dom.form.addEventListener("submit", handleSubmit);
  dom.createSiteBtn.addEventListener("click", switchToCreateMode);
  dom.resetFormBtn.addEventListener("click", () => fillForm(selectedSite()));
  dom.cancelEditBtn.addEventListener("click", switchToCreateMode);
  dom.refreshAllBtn.addEventListener("click", async () => {
    try {
      await loadSites();
      showToast("已刷新站点列表", "success");
    } catch (error) {
      showToast(error.message, "error");
    }
  });
  dom.searchInput.addEventListener("input", renderSiteList);
  dom.autoRefreshToggle.addEventListener("change", () => {
    state.autoRefresh = dom.autoRefreshToggle.checked;
    setupAutoRefresh();
  });

  dom.parseBtn.addEventListener("click", () => triggerAction("parse"));
  dom.startBtn.addEventListener("click", () => triggerAction("start"));
  dom.stopBtn.addEventListener("click", () => triggerAction("stop"));
  dom.deleteBtn.addEventListener("click", handleDelete);

  dom.logTabs.addEventListener("click", (event) => {
    const tab = event.target.closest("[data-log-tab]");
    if (!tab) {
      return;
    }
    state.activeLogTab = tab.dataset.logTab;
    renderLogs(state.logs);
  });
}

function setupAutoRefresh() {
  if (state.autoRefreshTimer) {
    window.clearInterval(state.autoRefreshTimer);
    state.autoRefreshTimer = null;
  }
  const interval = pollIntervalMs();
  if (!interval) {
    return;
  }
  state.autoRefreshTimer = window.setInterval(() => {
    const task = state.selectedSiteId
      ? refreshSelectedPanels({ silent: true, skipList: true })
      : loadSites();
    task.catch((error) => {
      showToast(error.message, "error");
    });
  }, interval);
}

async function bootstrap() {
  bindEvents();
  setupAutoRefresh();
  try {
    await loadSites({ keepSelection: false });
  } catch (error) {
    showToast(error.message, "error");
  }
}

bootstrap();
