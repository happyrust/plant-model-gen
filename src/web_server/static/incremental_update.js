// 增量版本水位页面（specs/022 候选4·方案A）
// 只读展示 Version Commit 存储的真实状态：Committed Watermark、锚点时间线、Commit Pending。
// 增量执行本身走 CLI（incremental-sesno / watch-incremental），页面不再提供触发按钮。
class IncrementalUpdateManager {
    constructor() {
        this.databases = [];
        this.refreshInterval = null;
    }

    async init() {
        await this.loadStatus();
        this.bindEvents();
        this.startAutoRefresh();
    }

    bindEvents() {
        document.getElementById('refresh-all').addEventListener('click', () => this.loadStatus());
    }

    // 加载各库水位状态
    async loadStatus() {
        try {
            const response = await fetch('/api/incremental/status');
            const data = await response.json();

            if (data.success) {
                this.databases = data.databases || [];
                this.updateStats(data);
                this.renderDatabases();
            } else {
                this.showNotification(data.error || '加载失败', 'error');
            }
        } catch (error) {
            console.error('加载增量状态失败:', error);
            this.showNotification('加载失败', 'error');
        }
    }

    updateStats(data) {
        const dbs = this.databases;
        const anchored = dbs.filter(db => db.last_anchor_sesno !== null && db.last_anchor_sesno !== undefined).length;
        document.getElementById('total-dbs').textContent = dbs.length;
        document.getElementById('total-anchored').textContent = anchored;
        document.getElementById('total-pending-commits').textContent = data.pending_commit_total ?? 0;
        document.getElementById('last-check').textContent = this.formatTime(data.last_check);
    }

    renderDatabases() {
        const container = document.getElementById('dbs-container');
        container.innerHTML = '';

        if (this.databases.length === 0) {
            container.innerHTML = '<div class="text-sm text-gray-500">暂无数据库记录（尚未解析任何库，或存储未初始化）。</div>';
            return;
        }

        this.databases.forEach(db => {
            container.appendChild(this.createDbCard(db));
        });
    }

    createDbCard(db) {
        const card = document.createElement('div');
        card.className = 'border rounded-lg p-4 hover:shadow-lg transition-shadow';

        const hasAnchor = db.last_anchor_sesno !== null && db.last_anchor_sesno !== undefined;
        const pendingCount = (db.pending_commits || []).length;
        const legacyAhead = db.legacy_max_sesno > db.committed_watermark;

        const anchorBadge = hasAnchor
            ? `<span class="ml-3 px-2 py-1 text-xs rounded-full ${db.last_anchor_source === 'full' ? 'source-full' : 'source-incremental'}">
                   <i class="fas fa-anchor mr-1"></i>${db.last_anchor_source || 'anchor'} @ sesno ${db.last_anchor_sesno}
               </span>`
            : `<span class="ml-3 px-2 py-1 text-xs rounded-full bg-gray-100 text-gray-600">无锚点（legacy 口径）</span>`;
        const pendingBadge = pendingCount > 0
            ? `<span class="ml-2 px-2 py-1 text-xs rounded-full pending-badge">
                   <i class="fas fa-triangle-exclamation mr-1"></i>Commit Pending × ${pendingCount}
               </span>`
            : '';

        card.innerHTML = `
            <div class="flex justify-between items-start">
                <div class="flex-1">
                    <div class="flex items-center mb-2 flex-wrap">
                        <h3 class="text-lg font-semibold text-gray-900">dbnum ${db.dbnum}</h3>
                        ${anchorBadge}
                        ${pendingBadge}
                    </div>
                    <div class="grid grid-cols-2 md:grid-cols-4 gap-4 text-sm">
                        <div>
                            <span class="text-gray-500">Committed Watermark:</span>
                            <span class="ml-1 font-medium text-gray-900">${db.committed_watermark}</span>
                        </div>
                        <div>
                            <span class="text-gray-500">记账口径 max sesno:</span>
                            <span class="ml-1 ${legacyAhead ? 'font-medium text-amber-600' : 'text-gray-700'}">${db.legacy_max_sesno}</span>
                        </div>
                        <div>
                            <span class="text-gray-500">最近锚点时间:</span>
                            <span class="ml-1 text-gray-700">${this.formatTime(db.last_anchored_at)}</span>
                        </div>
                        <div>
                            <span class="text-gray-500">增量执行:</span>
                            <span class="ml-1 text-gray-700 font-mono text-xs">incremental-sesno --dbnum ${db.dbnum}</span>
                        </div>
                    </div>
                    ${legacyAhead && pendingCount === 0 ? `
                        <div class="mt-2 text-xs text-amber-700">
                            记账口径领先锚点：可能存在早于锚定机制的写入，历史查询以锚点为准。
                        </div>` : ''}
                    ${pendingCount > 0 ? this.renderPendingList(db) : ''}
                </div>
                <div class="flex flex-col space-y-2 ml-4">
                    <button onclick="incrementalManager.triggerRun(${db.dbnum}, 'detect')"
                            class="px-3 py-1 text-sm bg-blue-600 text-white rounded hover:bg-blue-700">
                        <i class="fas fa-search mr-1"></i>检测(试跑)
                    </button>
                    <button onclick="incrementalManager.triggerRun(${db.dbnum}, 'sync')"
                            class="px-3 py-1 text-sm bg-green-600 text-white rounded hover:bg-green-700">
                        <i class="fas fa-sync mr-1"></i>增量同步
                    </button>
                    <button onclick="incrementalManager.showDetails(${db.dbnum})"
                            class="px-3 py-1 text-sm bg-gray-600 text-white rounded hover:bg-gray-700">
                        <i class="fas fa-timeline mr-1"></i>锚点时间线
                    </button>
                </div>
            </div>
        `;

        return card;
    }

    renderPendingList(db) {
        return `
            <div class="mt-3 pt-3 border-t">
                <div class="text-sm text-red-700 mb-2">
                    未恢复的 Version Commit（阻塞该库更高 sesno 提交，恢复：
                    <span class="font-mono text-xs">incremental-sesno --dbnum ${db.dbnum} ... --recover-pending</span>）
                </div>
                <div class="space-y-1">
                    ${db.pending_commits.map(p => `
                        <div class="flex items-center text-xs">
                            <span class="px-2 py-0.5 rounded pending-badge">${p.status}</span>
                            <span class="ml-2 text-gray-700">sesno → ${p.to_sesno}</span>
                            <span class="ml-2 text-gray-500 truncate">${p.last_error || ''}</span>
                            <span class="ml-auto text-gray-500">${this.formatTime(p.updated_at)}</span>
                        </div>
                    `).join('')}
                </div>
            </div>
        `;
    }

    // 触发一次增量运行（detect=只读试跑 / sync=真实落库），启动后轮询状态
    async triggerRun(dbnum, kind) {
        const endpoint = kind === 'sync'
            ? `/api/incremental/sync/${dbnum}`
            : `/api/incremental/detect/${dbnum}`;
        if (kind === 'sync' && !confirm(`确定对 dbnum ${dbnum} 触发增量同步（真实落库，固化 Version Anchor）？`)) {
            return;
        }
        try {
            const response = await fetch(endpoint, { method: 'POST' });
            const data = await response.json();
            if (!data.success) {
                this.showNotification(data.error || '触发失败', 'error');
                return;
            }
            this.showNotification(`已启动 ${kind === 'sync' ? '增量同步' : '检测试跑'}：${data.run_id}`, 'success');
            this.pollRun(data.run_id);
        } catch (error) {
            console.error('触发增量运行失败:', error);
            this.showNotification('触发失败', 'error');
        }
    }

    // 轮询一次增量运行直到终态
    async pollRun(runId) {
        const started = Date.now();
        const tick = async () => {
            try {
                const response = await fetch(`/api/incremental/task/${runId}`);
                const data = await response.json();
                if (data.success && data.run) {
                    const state = data.run.state;
                    if (state === 'running') {
                        if (Date.now() - started < 30 * 60 * 1000) {
                            setTimeout(tick, 3000);
                        }
                        return;
                    }
                    if (state === 'succeeded') {
                        this.showNotification(`运行 ${runId} 完成`, 'success');
                    } else {
                        this.showNotification(`运行 ${runId} 失败：${data.run.error || ''}`, 'error');
                    }
                    this.loadStatus();
                }
            } catch (error) {
                console.error('轮询运行状态失败:', error);
            }
        };
        setTimeout(tick, 2000);
    }

    // 锚点时间线详情
    async showDetails(dbnum) {
        try {
            const response = await fetch(`/api/incremental/site/${dbnum}`);
            const data = await response.json();

            if (data.success) {
                this.renderDetailModal(data);
                document.getElementById('detail-modal').classList.remove('hidden');
            } else {
                this.showNotification(data.error || '加载详情失败', 'error');
            }
        } catch (error) {
            console.error('加载详情失败:', error);
            this.showNotification('加载详情失败', 'error');
        }
    }

    renderDetailModal(data) {
        const modalTitle = document.getElementById('modal-title');
        const modalContent = document.getElementById('modal-content');

        modalTitle.textContent = `dbnum ${data.dbnum} - 锚点时间线（最近 ${data.anchors.length} 条）`;

        const pendingSection = (data.pending_commits || []).length > 0 ? `
            <div>
                <h4 class="font-semibold text-red-700 mb-2">Commit Pending（需人工恢复）</h4>
                <div class="border border-red-200 rounded p-2 space-y-1">
                    ${data.pending_commits.map(p => `
                        <div class="flex items-center text-xs">
                            <span class="px-2 py-0.5 rounded pending-badge">${p.status}</span>
                            <span class="ml-2 text-gray-700">sesno → ${p.to_sesno}</span>
                            <span class="ml-2 text-gray-500">${p.last_error || ''}</span>
                            <span class="ml-auto text-gray-500">${this.formatTime(p.updated_at)}</span>
                        </div>
                    `).join('')}
                </div>
            </div>` : '';

        modalContent.innerHTML = `
            <div class="space-y-4">
                ${pendingSection}
                <div>
                    <h4 class="font-semibold text-gray-900 mb-2">Version Anchor</h4>
                    <div class="max-h-96 overflow-y-auto border rounded p-2">
                        <table class="w-full text-sm">
                            <thead>
                                <tr class="border-b">
                                    <th class="text-left py-1">sesno</th>
                                    <th class="text-left py-1">区间</th>
                                    <th class="text-left py-1">来源</th>
                                    <th class="text-left py-1">锚定时间</th>
                                    <th class="text-right py-1">pe/att/uda/del</th>
                                    <th class="text-left py-1">fingerprint</th>
                                </tr>
                            </thead>
                            <tbody>
                                ${data.anchors.map(anchor => `
                                    <tr class="border-b hover:bg-gray-50">
                                        <td class="py-1 font-medium text-gray-900">${anchor.sesno}</td>
                                        <td class="py-1 text-gray-600">${anchor.from_sesno ?? '--'} → ${anchor.sesno}</td>
                                        <td class="py-1">
                                            <span class="px-2 py-0.5 text-xs rounded ${anchor.source === 'full' ? 'source-full' : 'source-incremental'}">
                                                ${anchor.source || '--'}
                                            </span>
                                        </td>
                                        <td class="py-1 text-gray-600">${this.formatTime(anchor.anchored_at)}</td>
                                        <td class="py-1 text-right text-gray-600">
                                            ${anchor.counts?.pe_rows ?? '-'} / ${anchor.counts?.att_rows ?? '-'} / ${anchor.counts?.uda_rows ?? '-'} / ${anchor.counts?.delete_count ?? '-'}
                                        </td>
                                        <td class="py-1 text-gray-500 font-mono text-xs">${anchor.fingerprint ? anchor.fingerprint.slice(0, 12) : 'legacy'}</td>
                                    </tr>
                                `).join('')}
                            </tbody>
                        </table>
                        ${data.anchors.length === 0 ? '<div class="text-sm text-gray-500 p-2">该库暂无锚点（Legacy 口径或从未增量）。</div>' : ''}
                    </div>
                </div>
            </div>
        `;
    }

    startAutoRefresh() {
        // 每30秒刷新一次
        this.refreshInterval = setInterval(() => {
            this.loadStatus();
        }, 30000);
    }

    stopAutoRefresh() {
        if (this.refreshInterval) {
            clearInterval(this.refreshInterval);
            this.refreshInterval = null;
        }
    }

    formatTime(timeStr) {
        if (!timeStr) return '--';
        const date = new Date(timeStr);
        if (isNaN(date.getTime())) return timeStr;
        const now = new Date();
        const diff = now - date;

        if (diff < 60000) return '刚刚';
        if (diff < 3600000) return `${Math.floor(diff / 60000)} 分钟前`;
        if (diff < 86400000) return `${Math.floor(diff / 3600000)} 小时前`;

        return date.toLocaleDateString('zh-CN') + ' ' + date.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' });
    }

    showNotification(message, type = 'info') {
        const colors = {
            'success': 'bg-green-500',
            'error': 'bg-red-500',
            'info': 'bg-blue-500',
            'warning': 'bg-amber-500'
        };

        const notification = document.createElement('div');
        notification.className = `fixed top-4 right-4 px-4 py-2 text-white rounded shadow-lg ${colors[type]} z-900`;
        notification.textContent = message;
        document.body.appendChild(notification);

        setTimeout(() => {
            notification.remove();
        }, 3000);
    }
}

// 全局方法
function closeDetailModal() {
    document.getElementById('detail-modal').classList.add('hidden');
}

// 初始化
const incrementalManager = new IncrementalUpdateManager();
document.addEventListener('DOMContentLoaded', () => {
    incrementalManager.init();
});
