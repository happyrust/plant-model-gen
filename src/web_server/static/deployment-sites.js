function deploymentSitesApp() {
    return {
        sites: [],
        currentPage: 1,
        totalPages: 1,
        totalItems: 0,
        perPage: 10,
        loading: false,

        searchQuery: '',
        statusFilter: '',
        regionFilter: '',

        selectedSite: null,
        showCreateModal: false,
        editingSiteId: null,
        form: null,

        init() {
            this.form = this.defaultForm();
            this.loadSites();
        },

        defaultConfig() {
            return {
                name: '默认配置',
                manual_db_nums: [],
                manual_refnos: [],
                enabled_nouns: null,
                excluded_nouns: null,
                debug_limit_per_noun_type: null,
                project_name: '',
                project_path: '',
                project_code: 1516,
                mdb_name: 'ALL',
                module: 'DESI',
                db_type: 'surrealdb',
                surreal_ns: 1516,
                db_ip: 'localhost',
                db_port: '8020',
                db_user: 'root',
                db_password: 'root',
                gen_model: true,
                gen_mesh: false,
                gen_spatial_tree: true,
                apply_boolean_operation: true,
                mesh_tol_ratio: 3.0,
                room_keyword: '-RM',
                target_sesno: null,
                meshes_path: null,
                export_json: false,
                export_parquet: true,
            };
        },

        defaultForm() {
            const backendUrl = window.location.origin || 'http://127.0.0.1:3100';
            const frontendUrl = backendUrl.replace(/:\d+$/, ':5173');
            const bindPort = Number(window.location.port || 3100);
            return {
                site_id: '',
                name: '',
                description: '',
                region: '',
                env: '',
                project_name: '',
                project_path: '',
                project_code: 1516,
                frontend_url: frontendUrl,
                backend_url: backendUrl,
                bind_host: '0.0.0.0',
                bind_port: bindPort,
                owner: '',
                health_url: '',
                notes: '',
                config: this.defaultConfig(),
            };
        },

        siteKey(site) {
            return site?.site_id || site?.id || '';
        },

        normalizeSite(site) {
            const base = this.defaultForm();
            const config = Object.assign(this.defaultConfig(), site?.config || {});
            const projectName = site?.project_name || config.project_name || '';
            const projectPath = site?.project_path || config.project_path || '';
            const projectCode = site?.project_code ?? config.project_code ?? base.project_code;
            config.project_name = projectName;
            config.project_path = projectPath;
            config.project_code = Number(projectCode || 0) || base.project_code;
            config.surreal_ns = Number(config.surreal_ns || config.project_code || base.project_code);

            return {
                site_id: site?.site_id || site?.id || '',
                name: site?.name || '',
                description: site?.description || '',
                region: site?.region || site?.env || '',
                env: site?.env || site?.region || '',
                project_name: projectName,
                project_path: projectPath,
                project_code: Number(projectCode || 0) || base.project_code,
                frontend_url: site?.frontend_url || '',
                backend_url: site?.backend_url || site?.url || '',
                bind_host: site?.bind_host || '0.0.0.0',
                bind_port: Number(site?.bind_port || 0) || base.bind_port,
                owner: site?.owner || '',
                health_url: site?.health_url || '',
                notes: site?.notes || '',
                status: site?.status || 'Configuring',
                last_seen_at: site?.last_seen_at || null,
                updated_at: site?.updated_at || null,
                created_at: site?.created_at || null,
                config,
                e3d_projects: Array.isArray(site?.e3d_projects) ? site.e3d_projects : [],
            };
        },

        syncConfigFields() {
            this.form.project_code = Number(this.form.project_code || 0);
            this.form.bind_port = Number(this.form.bind_port || 0);
            this.form.config.project_name = this.form.project_name || this.form.config.project_name;
            this.form.config.project_path = this.form.project_path || '';
            this.form.config.project_code = this.form.project_code || this.form.config.project_code || 0;
            this.form.config.surreal_ns = Number(this.form.config.surreal_ns || this.form.project_code || this.form.config.project_code || 0);
            this.form.config.name = this.form.config.name || this.form.name || '默认配置';
        },

        validateUrl(value, fieldName) {
            try {
                const parsed = new URL(value);
                if (!parsed.protocol.startsWith('http')) {
                    throw new Error('invalid protocol');
                }
            } catch (_error) {
                throw new Error(`${fieldName}格式不正确`);
            }
        },

        validateForm() {
            const requiredTextFields = [
                ['site_id', '站点 ID'],
                ['name', '站点名称'],
                ['region', '区域'],
                ['project_name', '项目'],
                ['bind_host', '监听 Host'],
            ];

            for (const [field, label] of requiredTextFields) {
                const value = (this.form[field] || '').toString().trim();
                if (!value) {
                    throw new Error(`${label}不能为空`);
                }
            }

            if (!Number.isInteger(Number(this.form.project_code)) || Number(this.form.project_code) <= 0) {
                throw new Error('project_code 必须为正整数');
            }
            if (!Number.isInteger(Number(this.form.bind_port)) || Number(this.form.bind_port) <= 0 || Number(this.form.bind_port) > 65535) {
                throw new Error('监听 Port 必须为 1-65535 之间的整数');
            }

            this.validateUrl(this.form.frontend_url, '前端地址');
            this.validateUrl(this.form.backend_url, '后端地址');
            if (this.form.health_url && this.form.health_url.trim()) {
                this.validateUrl(this.form.health_url, '健康检查地址');
            }
        },

        buildPayload() {
            this.syncConfigFields();
            const projectPath = (this.form.project_path || '').trim();
            return {
                site_id: (this.form.site_id || '').trim(),
                name: (this.form.name || '').trim(),
                description: (this.form.description || '').trim() || null,
                region: (this.form.region || '').trim(),
                env: (this.form.env || '').trim() || null,
                project_name: (this.form.project_name || '').trim(),
                project_path: projectPath || null,
                project_code: Number(this.form.project_code),
                frontend_url: (this.form.frontend_url || '').trim(),
                backend_url: (this.form.backend_url || '').trim(),
                bind_host: (this.form.bind_host || '').trim(),
                bind_port: Number(this.form.bind_port),
                owner: (this.form.owner || '').trim() || null,
                health_url: (this.form.health_url || '').trim() || null,
                notes: (this.form.notes || '').trim() || null,
                selected_projects: projectPath ? [projectPath] : [],
                config: this.form.config,
            };
        },

        buildQueryParams() {
            const params = new URLSearchParams({
                page: String(this.currentPage),
                per_page: String(this.perPage),
            });
            if (this.searchQuery && this.searchQuery.trim()) {
                params.append('q', this.searchQuery.trim());
            }
            if (this.statusFilter && this.statusFilter.trim()) {
                params.append('status', this.statusFilter.trim());
            }
            if (this.regionFilter && this.regionFilter.trim()) {
                params.append('region', this.regionFilter.trim());
            }
            return params;
        },

        adminHeaders() {
            const headers = { 'Content-Type': 'application/json' };
            const token = window.localStorage?.getItem('admin_token');
            if (token) {
                headers.Authorization = `Bearer ${token}`;
            }
            return headers;
        },

        unwrapEnvelope(data) {
            if (data && typeof data === 'object' && Object.prototype.hasOwnProperty.call(data, 'data')) {
                return data.data || {};
            }
            return data || {};
        },

        async readJsonSafe(response) {
            const text = await response.text();
            if (!text) {
                return {};
            }
            try {
                return JSON.parse(text);
            } catch (_error) {
                return { raw: text };
            }
        },

        async extractError(response, fallbackMessage) {
            const data = await this.readJsonSafe(response);
            return (
                data?.error ||
                data?.message ||
                data?.raw ||
                `${fallbackMessage}（HTTP ${response.status}）`
            );
        },

        async loadSites() {
            this.loading = true;
            const previousSelectedId = this.siteKey(this.selectedSite);
            try {
                const response = await fetch(`/api/admin/registry/sites?${this.buildQueryParams().toString()}`, {
                    headers: this.adminHeaders(),
                });
                if (!response.ok) {
                    throw new Error(await this.extractError(response, '加载站点列表失败'));
                }
                const data = this.unwrapEnvelope(await this.readJsonSafe(response));
                this.sites = Array.isArray(data.items) ? data.items.map((site) => this.normalizeSite(site)) : [];
                this.totalItems = Number(data.total || this.sites.length || 0);
                this.totalPages = Number(data.pages || 1);
                this.currentPage = Number(data.page || this.currentPage || 1);

                if (!this.selectedSite && this.sites.length > 0) {
                    this.selectedSite = this.sites[0];
                } else if (previousSelectedId) {
                    const matched = this.sites.find((site) => this.siteKey(site) === previousSelectedId);
                    if (matched) {
                        this.selectedSite = matched;
                    }
                }
            } catch (error) {
                console.error('加载站点列表失败:', error);
                this.showError(error.message || '加载站点列表失败');
            } finally {
                this.loading = false;
            }
        },

        searchSites() {
            this.currentPage = 1;
            this.loadSites();
        },

        filterSites() {
            this.currentPage = 1;
            this.loadSites();
        },

        refreshSites() {
            this.loadSites();
        },

        openCreateModal() {
            this.editingSiteId = null;
            this.form = this.defaultForm();
            this.showCreateModal = true;
        },

        closeModal() {
            this.showCreateModal = false;
            this.editingSiteId = null;
            this.form = this.defaultForm();
        },

        async openImportDialog() {
            const pathInput = window.prompt('请输入 DbOption.toml 路径（留空默认 db_options/DbOption.toml）', '');
            if (pathInput === null) {
                return;
            }
            const frontendUrl = window.prompt('请输入前端地址（必填，用于站点配置区域）', this.form?.frontend_url || 'http://127.0.0.1:5173');
            if (frontendUrl === null) {
                return;
            }
            const backendUrl = window.prompt('请输入后端地址（留空则按 DbOption / 当前端口推导）', this.form?.backend_url || window.location.origin);
            if (backendUrl === null) {
                return;
            }
            const siteId = window.prompt('请输入站点 ID（留空则按项目名 + 端口自动生成）', '');
            if (siteId === null) {
                return;
            }
            const region = window.prompt('请输入区域（留空则回退 DbOption.location）', this.form?.region || '');
            if (region === null) {
                return;
            }

            const payload = {};
            if (pathInput.trim()) payload.path = pathInput.trim();
            if (frontendUrl.trim()) payload.frontend_url = frontendUrl.trim();
            if (backendUrl.trim()) payload.backend_url = backendUrl.trim();
            if (siteId.trim()) payload.site_id = siteId.trim();
            if (region.trim()) payload.region = region.trim();

            try {
                const response = await fetch('/api/admin/registry/import-dboption', {
                    method: 'POST',
                    headers: this.adminHeaders(),
                    body: JSON.stringify(payload),
                });
                if (!response.ok) {
                    throw new Error(await this.extractError(response, '导入站点失败'));
                }
                const data = this.unwrapEnvelope(await this.readJsonSafe(response));
                const item = data ? this.normalizeSite(data) : null;
                await this.loadSites();
                if (item) {
                    this.selectedSite = item;
                }
                this.showSuccess(data?.message || '站点导入成功');
            } catch (error) {
                console.error('导入站点失败:', error);
                this.showError(error.message || '导入站点失败');
            }
        },

        async viewSiteDetail(site) {
            const siteId = this.siteKey(site);
            if (!siteId) {
                this.showError('无效的站点 ID');
                return;
            }
            try {
                const response = await fetch(`/api/admin/registry/sites/${encodeURIComponent(siteId)}`, {
                    headers: this.adminHeaders(),
                });
                if (!response.ok) {
                    throw new Error(await this.extractError(response, '加载站点详情失败'));
                }
                const data = this.unwrapEnvelope(await this.readJsonSafe(response));
                this.selectedSite = this.normalizeSite(data);
            } catch (error) {
                console.error('加载站点详情失败:', error);
                this.showError(error.message || '加载站点详情失败');
            }
        },

        async editSite(site) {
            await this.viewSiteDetail(site);
            if (!this.selectedSite) {
                return;
            }
            this.editingSiteId = this.siteKey(this.selectedSite);
            this.form = this.normalizeSite(this.selectedSite);
            this.showCreateModal = true;
        },

        async submitSiteForm() {
            try {
                this.validateForm();
                const payload = this.buildPayload();
                const editingId = this.editingSiteId;
                const url = editingId
                    ? `/api/admin/registry/sites/${encodeURIComponent(editingId)}`
                    : '/api/admin/registry/sites';
                const method = editingId ? 'PUT' : 'POST';
                const response = await fetch(url, {
                    method,
                    headers: this.adminHeaders(),
                    body: JSON.stringify(payload),
                });
                if (!response.ok) {
                    throw new Error(await this.extractError(response, editingId ? '更新站点失败' : '创建站点失败'));
                }
                const data = this.unwrapEnvelope(await this.readJsonSafe(response));
                const item = data ? this.normalizeSite(data) : null;
                this.closeModal();
                await this.loadSites();
                if (item) {
                    this.selectedSite = item;
                }
                this.showSuccess(editingId ? '站点更新成功' : '站点创建成功');
            } catch (error) {
                console.error('保存站点失败:', error);
                this.showError(error.message || '保存站点失败');
            }
        },

        async deleteSite(site) {
            const siteId = this.siteKey(site);
            const displayName = site?.name || siteId;
            if (!siteId) {
                this.showError('无效的站点 ID');
                return;
            }
            if (!window.confirm(`确定要删除站点“${displayName}”吗？`)) {
                return;
            }
            try {
                const response = await fetch(`/api/admin/registry/sites/${encodeURIComponent(siteId)}`, {
                    method: 'DELETE',
                    headers: this.adminHeaders(),
                });
                if (!response.ok) {
                    if (response.status === 409) {
                        throw new Error('当前运行中的站点不能直接删除，请先停掉对应 web_server 进程');
                    }
                    throw new Error(await this.extractError(response, '删除站点失败'));
                }
                await this.loadSites();
                if (this.siteKey(this.selectedSite) === siteId) {
                    this.selectedSite = this.sites[0] || null;
                }
                this.showSuccess(`站点“${displayName}”已删除`);
            } catch (error) {
                console.error('删除站点失败:', error);
                this.showError(error.message || '删除站点失败');
            }
        },

        async refreshSiteStatus(site) {
            const siteId = this.siteKey(site);
            if (!siteId) {
                this.showError('无效的站点 ID');
                return;
            }
            try {
                const response = await fetch(`/api/admin/registry/sites/${encodeURIComponent(siteId)}/healthcheck`, {
                    method: 'POST',
                    headers: this.adminHeaders(),
                    body: JSON.stringify({}),
                });
                if (!response.ok) {
                    throw new Error(await this.extractError(response, '站点探活失败'));
                }
                const data = this.unwrapEnvelope(await this.readJsonSafe(response));
                await this.loadSites();
                if (this.selectedSite && this.siteKey(this.selectedSite) === siteId) {
                    this.selectedSite = this.normalizeSite(data?.item || this.selectedSite);
                }
                this.showSuccess(data?.healthy ? '站点健康检查成功' : '站点健康检查失败');
            } catch (error) {
                console.error('站点探活失败:', error);
                this.showError(error.message || '站点探活失败');
            }
        },

        async copyAddress(url) {
            if (!url) {
                this.showInfo('当前站点没有可复制的地址');
                return;
            }
            try {
                if (navigator.clipboard?.writeText) {
                    await navigator.clipboard.writeText(url);
                } else {
                    const textarea = document.createElement('textarea');
                    textarea.value = url;
                    textarea.style.position = 'fixed';
                    textarea.style.opacity = '0';
                    document.body.appendChild(textarea);
                    textarea.select();
                    document.execCommand('copy');
                    textarea.remove();
                }
                this.showSuccess('地址已复制到剪贴板');
            } catch (error) {
                console.error('复制地址失败:', error);
                this.showError('复制地址失败');
            }
        },

        formatDate(dateString) {
            if (!dateString) {
                return '-';
            }
            try {
                return new Date(dateString).toLocaleString('zh-CN');
            } catch (_error) {
                return dateString;
            }
        },

        formatConfig(config) {
            try {
                return JSON.stringify(config || {}, null, 2);
            } catch (_error) {
                return '{}';
            }
        },

        getStatusColor(status) {
            switch ((status || '').toString()) {
                case 'Running':
                    return 'bg-green-100 text-green-700';
                case 'Offline':
                    return 'bg-amber-100 text-amber-700';
                case 'Failed':
                    return 'bg-red-100 text-red-700';
                case 'Stopped':
                    return 'bg-gray-100 text-gray-700';
                case 'Deploying':
                    return 'bg-blue-100 text-blue-700';
                default:
                    return 'bg-slate-100 text-slate-700';
            }
        },

        getStatusText(status) {
            switch ((status || '').toString()) {
                case 'Running':
                    return '运行中';
                case 'Offline':
                    return '离线';
                case 'Failed':
                    return '失败';
                case 'Stopped':
                    return '已停止';
                case 'Deploying':
                    return '部署中';
                case 'Configuring':
                    return '配置中';
                default:
                    return status || '-';
            }
        },

        showSuccess(message) {
            this.showNotification(message, 'success');
        },

        showError(message) {
            this.showNotification(message, 'error');
        },

        showInfo(message) {
            this.showNotification(message, 'info');
        },

        showNotification(message, type = 'info') {
            const colors = {
                success: 'bg-green-100 text-green-800 border border-green-200',
                error: 'bg-red-100 text-red-800 border border-red-200',
                info: 'bg-blue-100 text-blue-800 border border-blue-200',
                warning: 'bg-amber-100 text-amber-800 border border-amber-200',
            };
            const icons = {
                success: 'fa-check-circle',
                error: 'fa-circle-exclamation',
                info: 'fa-circle-info',
                warning: 'fa-triangle-exclamation',
            };
            const notification = document.createElement('div');
            notification.className = `fixed top-4 right-4 z-50 max-w-md rounded-lg px-4 py-3 shadow-lg ${colors[type] || colors.info}`;
            notification.innerHTML = `
                <div class="flex items-start gap-3">
                    <i class="fas ${icons[type] || icons.info} mt-0.5"></i>
                    <div class="flex-1 text-sm leading-5 break-words">${message}</div>
                    <button type="button" class="text-gray-400 hover:text-gray-600">
                        <i class="fas fa-times"></i>
                    </button>
                </div>
            `;
            const closeButton = notification.querySelector('button');
            closeButton?.addEventListener('click', () => notification.remove());
            document.body.appendChild(notification);
            window.setTimeout(() => notification.remove(), 3200);
        },
    };
}
