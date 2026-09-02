const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

let selectedGroupId = null;
let connected = false;

// ========== Helpers ==========

async function invokeCmd(cmd, args) {
    try {
        const result = await invoke(cmd, args);
        return { ok: true, data: result };
    } catch (e) {
        console.error(`[${cmd}] error:`, e);
        return { ok: false, error: String(e) };
    }
}

/**
 * Decode base64 content: try UTF-8 decode, fallback to escaped display.
 */
function decodeContent(base64Str) {
    if (!base64Str) return '';
    try {
        const binaryStr = atob(base64Str);
        const bytes = new Uint8Array(binaryStr.length);
        for (let i = 0; i < binaryStr.length; i++) {
            bytes[i] = binaryStr.charCodeAt(i);
        }
        // Try UTF-8 decode
        const decoder = new TextDecoder('utf-8', { fatal: true });
        return decoder.decode(bytes);
    } catch (e) {
        // Fallback: try UTF-8 from the base64 bytes directly
        try {
            const bytes = Uint8Array.from(atob(base64Str), c => c.charCodeAt(0));
            return new TextDecoder('utf-8').decode(bytes);
        } catch (e2) {
            return '[解码失败: ' + escape(base64Str.substring(0, 50)) + ']';
        }
    }
}

/**
 * Format timestamp to readable time string.
 */
function formatTime(ts) {
    if (!ts) return '';
    const d = new Date(ts);
    if (isNaN(d.getTime())) return String(ts);
    return d.toLocaleString('zh-CN', { hour12: false });
}

function setStatus(text, isConnected) {
    const el = document.getElementById('status');
    el.textContent = text;
    if (isConnected !== undefined) {
        connected = isConnected;
    }
}

function showMainPanel() {
    document.getElementById('login-panel').style.display = 'none';
    document.getElementById('main-panel').style.display = '';
}

function showLoginPanel() {
    document.getElementById('login-panel').style.display = '';
    document.getElementById('main-panel').style.display = 'none';
}

// ========== Login Flow ==========

document.getElementById('send-code-btn').addEventListener('click', async () => {
    const phone = document.getElementById('phone').value.trim();
    if (!phone || phone.length < 11) {
        alert('请输入正确的手机号');
        return;
    }
    const btn = document.getElementById('send-code-btn');
    btn.disabled = true;
    btn.textContent = '发送中...';

    const res = await invokeCmd('send_sms_code', {
        phone: phone,
        countryCode: 86,
        gt4Dto: {}
    });

    btn.disabled = false;
    btn.textContent = '发送验证码';

    if (res.ok) {
        alert('验证码已发送到手机，请注意查收');
    } else {
        alert('发送失败: ' + res.error);
    }
});

document.getElementById('login-btn').addEventListener('click', async () => {
    const phone = document.getElementById('phone').value.trim();
    const code = document.getElementById('code').value.trim();
    if (!phone || !code) {
        alert('请填写手机号和验证码');
        return;
    }

    const btn = document.getElementById('login-btn');
    btn.disabled = true;
    btn.textContent = '登录中...';

    const res = await invokeCmd('login', {
        phone: phone,
        countryCode: 86,
        validateToken: code
    });

    btn.disabled = false;
    btn.textContent = '登录';

    if (res.ok) {
        showMainPanel();
        loadGroups();
        checkConnectionStatus();
    } else {
        alert('登录失败: ' + res.error);
    }
});

// ========== Group List ==========

async function loadGroups() {
    const list = document.getElementById('group-list');
    list.innerHTML = '<p class="loading" style="padding:1rem;text-align:center;color:var(--text-secondary)">加载中...</p>';

    const res = await invokeCmd('fetch_group_list', {});
    if (!res.ok) {
        list.innerHTML = '<p style="padding:1rem;text-align:center;color:var(--accent)">加载失败，请重新连接</p>';
        return;
    }

    const groups = res.data;
    if (!Array.isArray(groups) || groups.length === 0) {
        list.innerHTML = '<p style="padding:1rem;text-align:center;color:var(--text-secondary)">暂无监控群，请先在左侧点击"连接聊天"</p>';
        return;
    }

    renderGroupList(groups);
}

function renderGroupList(groups) {
    const list = document.getElementById('group-list');
    list.innerHTML = groups.map(g => {
        const monitored = g.monitored === 1 || g.monitored === true;
        return `
        <div class="group-item ${monitored ? 'monitored' : ''}" data-id="${g.group_id}">
            <span>${escapeHtml(g.name || '未命名群')}</span>
            <span class="count">${g.member_count || 0}人</span>
        </div>`;
    }).join('');

    list.querySelectorAll('.group-item').forEach(el => {
        el.addEventListener('click', () => {
            selectedGroupId = parseInt(el.dataset.id);
            document.querySelectorAll('.group-item').forEach(e => e.classList.remove('selected'));
            el.classList.add('selected');
            document.getElementById('selected-group-name').textContent = el.querySelector('span').textContent;
            loadMessages(selectedGroupId);
        });
    });
}

function escapeHtml(str) {
    const div = document.createElement('div');
    div.appendChild(document.createTextNode(str));
    return div.innerHTML;
}

// ========== Message Loading ==========

async function loadMessages(groupId) {
    const list = document.getElementById('message-list');
    list.innerHTML = '<p class="loading" style="padding:1rem;text-align:center;color:var(--text-secondary)">加载中...</p>';

    const res = await invokeCmd('get_messages', { groupId: groupId, limit: 50, offset: 0 });
    if (!res.ok) {
        list.innerHTML = '<p style="padding:1rem;text-align:center;color:var(--accent)">加载失败</p>';
        return;
    }

    const messages = res.data;
    if (!Array.isArray(messages) || messages.length === 0) {
        list.innerHTML = '<p style="padding:1rem;text-align:center;color:var(--text-secondary)">暂无消息</p>';
        return;
    }

    list.innerHTML = '';
    // Render in chronological order (oldest first)
    messages.sort((a, b) => a.send_time - b.send_time);
    messages.forEach(msg => {
        appendMessageRaw(msg);
    });
    list.scrollTop = list.scrollHeight;
}

function appendMessageRaw(msg) {
    const list = document.getElementById('message-list');
    const div = document.createElement('div');
    div.className = 'message';
    const contentText = decodeContent(msg.content_b64 || msg.content);
    const timeStr = formatTime(msg.send_time);
    div.innerHTML = `<span class="time">${timeStr}</span> <span class="content">${escapeHtml(contentText)}</span>`;
    list.appendChild(div);
    list.scrollTop = list.scrollHeight;
}

function appendMessage(payload) {
    if (!payload || payload.group_id !== selectedGroupId) return;
    appendMessageRaw(payload);
}

// ========== Connect / Disconnect ==========

async function checkConnectionStatus() {
    // Check if we have stored credentials
    // The backend tracks connected state; we infer from whether connect succeeded previously
    // For now, assume not connected until user clicks connect
    updateConnectButtons(false);
}

function updateConnectButtons(isConnected) {
    const connectBtn = document.getElementById('connect-btn');
    const disconnectBtn = document.getElementById('disconnect-btn');
    if (isConnected) {
        connectBtn.style.display = 'none';
        disconnectBtn.style.display = '';
        setStatus('已连接', true);
    } else {
        connectBtn.style.display = '';
        disconnectBtn.style.display = 'none';
        setStatus('未连接', false);
    }
}

document.getElementById('connect-btn').addEventListener('click', async () => {
    const btn = document.getElementById('connect-btn');
    btn.disabled = true;
    btn.textContent = '连接中...';
    setStatus('连接中...', false);

    const res = await invokeCmd('connect_chat', {});

    if (res.ok) {
        updateConnectButtons(true);
        loadGroups(); // Reload groups after connect (to refresh monitored list)
    } else {
        btn.disabled = false;
        btn.textContent = '连接聊天';
        alert('连接失败: ' + res.error);
        setStatus('连接失败', false);
    }
});

document.getElementById('disconnect-btn').addEventListener('click', async () => {
    const res = await invokeCmd('disconnect_chat', {});
    if (res.ok) {
        updateConnectButtons(false);
        document.getElementById('message-list').innerHTML = '<p style="padding:1rem;text-align:center;color:var(--text-secondary)">已断开连接</p>';
        selectedGroupId = null;
        document.getElementById('selected-group-name').textContent = '选择一个群';
        document.querySelectorAll('.group-item').forEach(e => e.classList.remove('selected'));
    } else {
        alert('断开连接失败: ' + res.error);
    }
});

// ========== New Message Event Listener ==========

listen('new_message', (event) => {
    if (event && event.payload) {
        appendMessage(event.payload);
    }
});

// ========== Logout ==========

document.getElementById('logout-btn')?.addEventListener('click', async () => {
    await invokeCmd('logout', {});
    showLoginPanel();
    selectedGroupId = null;
    document.getElementById('selected-group-name').textContent = '选择一个群';
    document.getElementById('group-list').innerHTML = '';
    document.getElementById('message-list').innerHTML = '';
    updateConnectButtons(false);
});

// ========== Init ==========

// Check if we can start already logged in (future enhancement: remember session)
// For now, always show login panel
showLoginPanel();
