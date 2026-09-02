const { invoke } = window.__TAURI__.core;

let selectedGroupId = null;

async function loadGroups() {
    try {
        const groups = await invoke('fetch_group_list');
        renderGroupList(groups);
    } catch (e) {
        console.error('Failed to load groups:', e);
    }
}

function renderGroupList(groups) {
    const list = document.getElementById('group-list');
    list.innerHTML = groups.map(g => `
        <div class="group-item ${g.monitored ? 'monitored' : ''}" data-id="${g.group_id}">
            <span>${g.name}</span>
            <span class="count">${g.member_count}</span>
        </div>
    `).join('');

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

async function loadMessages(groupId) {
    // Phase 2: load from database
    document.getElementById('message-list').innerHTML = '<p class="loading">加载中...</p>';
}

document.getElementById('send-code-btn').addEventListener('click', async () => {
    const phone = document.getElementById('phone').value;
    // Phase 2: call send_sms_code
    alert('Phase 2: 发送验证码功能');
});

document.getElementById('login-btn').addEventListener('click', async () => {
    const phone = document.getElementById('phone').value;
    const code = document.getElementById('code').value;
    // Phase 2: call login
    alert('Phase 2: 登录功能');
});

document.getElementById('connect-btn').addEventListener('click', async () => {
    try {
        await invoke('connect_chat');
        document.getElementById('status').textContent = '已连接';
    } catch (e) {
        document.getElementById('status').textContent = '连接失败';
    }
});

// Listen for new message events
window.__TAURI__.event.listen('new_message', (event) => {
    if (event.payload.group_id === selectedGroupId) {
        appendMessage(event.payload);
    }
});

function appendMessage(msg) {
    const list = document.getElementById('message-list');
    const time = new Date(msg.send_time).toLocaleTimeString();
    const div = document.createElement('div');
    div.className = 'message';
    div.innerHTML = `<span class="time">${time}</span> <span class="content">${msg.content}</span>`;
    list.appendChild(div);
    list.scrollTop = list.scrollHeight;
}

// Init
loadGroups();
