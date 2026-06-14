(function () {
  "use strict";

  var csrf = (document.querySelector('meta[name="csrf"]') || {}).content || "";
  var PRIVILEGED = ["deploy", "babylon", "operator"];
  var overviewCache = null;

  function el(tag, opts) {
    var node = document.createElement(tag);
    if (opts) {
      if (opts.text != null) node.textContent = String(opts.text);
      if (opts.cls) node.className = opts.cls;
      if (opts.attrs) {
        Object.keys(opts.attrs).forEach(function (k) {
          node.setAttribute(k, opts.attrs[k]);
        });
      }
    }
    return node;
  }

  function clear(node) {
    while (node.firstChild) node.removeChild(node.firstChild);
  }

  function notice(msg, isError) {
    var n = document.getElementById("notice");
    n.textContent = msg;
    n.hidden = false;
    n.style.borderLeftColor = isError ? "" : "var(--lapis)";
    n.style.background = isError ? "" : "rgba(42, 77, 143, 0.1)";
    n.style.color = isError ? "" : "var(--lapis-deep)";
  }

  function clearNotice() {
    var n = document.getElementById("notice");
    n.hidden = true;
    n.textContent = "";
  }

  function fmtBytes(n) {
    if (!n) return "0 B";
    var units = ["B", "KB", "MB", "GB", "TB"];
    var i = 0;
    var v = n;
    while (v >= 1024 && i < units.length - 1) {
      v /= 1024;
      i++;
    }
    return (i === 0 ? v : v.toFixed(1)) + " " + units[i];
  }

  function fmtAge(ts) {
    if (!ts) return "never";
    var now = Math.floor(Date.now() / 1000);
    var d = now - Math.floor(ts / 1000);
    if (d < 0) d = 0;
    if (d < 60) return d + "s ago";
    if (d < 3600) return Math.floor(d / 60) + "m ago";
    if (d < 86400) return Math.floor(d / 3600) + "h ago";
    return Math.floor(d / 86400) + "d ago";
  }

  async function api(url, method, payload) {
    var opts = { method: method, headers: {} };
    if (method === "POST") {
      opts.headers["Content-Type"] = "application/json";
      opts.headers["X-Babylon-CSRF"] = csrf;
      opts.body = JSON.stringify(payload || {});
    }
    var resp = await fetch(url, opts);
    var text = await resp.text();
    var data = null;
    if (text) {
      try {
        data = JSON.parse(text);
      } catch (e) {
        data = null;
      }
    }
    if (!resp.ok) {
      var detail = data && data.error ? data.error : text || ("HTTP " + resp.status);
      var err = new Error(detail);
      err.status = resp.status;
      throw err;
    }
    return data;
  }

  function renderHealth(o) {
    var dot = document.getElementById("health-dot");
    var txt = document.getElementById("health-text");
    var ok = o.health && o.health.ok && o.health.ready;
    dot.className = "dot " + (ok ? "on" : "off");
    txt.textContent = ok ? "healthy" : (o.health && o.health.ok ? "degraded" : "down");
    document.getElementById("pin").textContent = "pin " + (o.pin || "—");
  }

  function setStat(id, val) {
    document.getElementById(id).textContent = String(val);
  }

  function renderStats(o) {
    var s = o.stats || {};
    setStat("stat-agents", s.agents != null ? s.agents : 0);
    setStat("stat-channels", s.channels != null ? s.channels : 0);
    setStat("stat-messages", s.messages != null ? s.messages : 0);
    setStat("stat-questions", (o.open_questions || []).length);
    setStat("stat-tasks", (o.open_tasks || []).length);
    document.getElementById("stat-db").textContent = fmtBytes(o.db_bytes || 0);
  }

  function statusCell(online) {
    var wrap = el("span", { cls: "status-cell" });
    wrap.appendChild(el("span", { cls: "dot " + (online ? "on" : "off"), attrs: { "aria-hidden": "true" } }));
    wrap.appendChild(el("span", { text: online ? "online" : "offline" }));
    return wrap;
  }

  function renderAgents(o) {
    var body = document.getElementById("agents-body");
    clear(body);
    var agents = o.agents || [];
    if (!agents.length) {
      var tr = el("tr");
      var td = el("td", { text: "no agents registered", cls: "empty", attrs: { colspan: "5" } });
      tr.appendChild(td);
      body.appendChild(tr);
      return;
    }
    agents.forEach(function (a) {
      var tr = el("tr");
      tr.appendChild(el("td", { text: a.handle, cls: "mono" }));
      tr.appendChild(el("td", { text: a.kind }));
      tr.appendChild(el("td", { text: a.role || "—" }));
      tr.appendChild(el("td", { text: fmtAge(a.last_seen) }));
      var st = el("td", { cls: "t-right" });
      st.appendChild(statusCell(!!a.online));
      tr.appendChild(st);
      body.appendChild(tr);
    });
  }

  function renderChannels(o) {
    var body = document.getElementById("channels-body");
    clear(body);
    var channels = o.channels || [];
    if (!channels.length) {
      var tr = el("tr");
      tr.appendChild(el("td", { text: "no channels", cls: "empty", attrs: { colspan: "6" } }));
      body.appendChild(tr);
    }
    channels.forEach(function (c) {
      var row = el("tr");
      row.appendChild(el("td", { text: c.name, cls: "mono" }));
      row.appendChild(el("td", { text: c.topic || "—" }));
      row.appendChild(el("td", { text: c.member_count, cls: "t-right" }));
      row.appendChild(el("td", { text: c.message_count, cls: "t-right" }));
      var stateTd = el("td");
      stateTd.appendChild(el("span", {
        text: c.archived ? "archived" : c.kind,
        cls: c.archived ? "tag archived" : "tag"
      }));
      row.appendChild(stateTd);
      row.appendChild(el("td", { text: fmtAge(c.last_activity_ts), cls: "t-right" }));
      body.appendChild(row);
    });

    var nonDm = channels.filter(function (c) {
      return c.name.indexOf("dm:") !== 0 && !c.archived;
    });
    fillChannelSelect("post-channel", nonDm);
    fillChannelSelect("archive-channel", nonDm);
  }

  function fillChannelSelect(id, channels) {
    var sel = document.getElementById(id);
    var prev = sel.value;
    clear(sel);
    channels.forEach(function (c) {
      var opt = el("option", { text: c.name });
      opt.value = c.name;
      sel.appendChild(opt);
    });
    if (prev) sel.value = prev;
  }

  function renderFeed(id, items, emptyText) {
    var ul = document.getElementById(id);
    clear(ul);
    if (!items || !items.length) {
      ul.appendChild(el("li", { text: emptyText, cls: "feed-empty" }));
      return;
    }
    items.forEach(function (m) {
      var li = el("li");
      var meta = el("span", { cls: "feed-meta" });
      var to = m.to && m.to.length ? " → " + m.to.join(", ") : "";
      meta.textContent = "#" + m.ch + " · " + m.from + " · " + fmtAge(m.ts) + to;
      li.appendChild(meta);
      li.appendChild(el("span", { text: m.sum, cls: "feed-sum" }));
      ul.appendChild(li);
    });
  }

  function render(o) {
    overviewCache = o;
    renderHealth(o);
    renderStats(o);
    renderAgents(o);
    renderChannels(o);
    renderFeed("questions-list", o.open_questions, "no open questions");
    renderFeed("tasks-list", o.open_tasks, "no open tasks");
  }

  async function refresh() {
    try {
      var o = await api("/api/overview", "GET");
      render(o);
    } catch (e) {
      notice("Failed to load overview: " + e.message, true);
    }
  }

  function showToken(handle, token) {
    var box = document.getElementById("token-reveal");
    var val = document.getElementById("token-value");
    document.getElementById("token-handle").textContent = handle;
    val.dataset.token = token;
    val.textContent = "•".repeat(Math.min(token.length, 44));
    val.classList.add("masked");
    document.getElementById("token-toggle").textContent = "Reveal";
    box.hidden = false;
    box.scrollIntoView({ behavior: "smooth", block: "nearest" });
  }

  function bindTokenReveal() {
    var val = document.getElementById("token-value");
    document.getElementById("token-toggle").addEventListener("click", function () {
      var btn = this;
      if (val.classList.contains("masked")) {
        val.textContent = val.dataset.token || "";
        val.classList.remove("masked");
        btn.textContent = "Hide";
      } else {
        val.textContent = "•".repeat(Math.min((val.dataset.token || "").length, 44));
        val.classList.add("masked");
        btn.textContent = "Reveal";
      }
    });
    document.getElementById("token-copy").addEventListener("click", async function () {
      var btn = this;
      try {
        await navigator.clipboard.writeText(val.dataset.token || "");
        btn.textContent = "Copied";
        setTimeout(function () { btn.textContent = "Copy"; }, 1500);
      } catch (e) {
        notice("Copy failed — reveal and copy manually.", true);
      }
    });
    document.getElementById("token-dismiss").addEventListener("click", function () {
      var box = document.getElementById("token-reveal");
      box.hidden = true;
      val.dataset.token = "";
      val.textContent = "";
    });
  }

  function confirmPrivileged(handle, verb) {
    if (PRIVILEGED.indexOf(handle) !== -1) {
      return window.confirm('"' + handle + '" is a privileged identity. Really ' + verb + ' its token?');
    }
    return true;
  }

  async function submitForm(form, fn) {
    form.addEventListener("submit", async function (ev) {
      ev.preventDefault();
      clearNotice();
      var btn = form.querySelector('button[type="submit"]');
      if (btn) btn.disabled = true;
      try {
        await fn();
      } catch (e) {
        notice(e.message, true);
      } finally {
        if (btn) btn.disabled = false;
      }
    });
  }

  function wireForms() {
    submitForm(document.getElementById("form-mint"), async function () {
      var handle = document.getElementById("mint-handle").value.trim();
      var kind = document.getElementById("mint-kind").value;
      var res = await api("/api/tokens/mint", "POST", { handle: handle, kind: kind });
      showToken(res.handle, res.token);
      notice("Minted token for " + res.handle + ".", false);
      document.getElementById("mint-handle").value = "";
      await refresh();
    });

    submitForm(document.getElementById("form-rotate"), async function () {
      var handle = document.getElementById("rotate-handle").value.trim();
      if (!window.confirm('Rotate token for "' + handle + '"? The current token stops working.')) return;
      if (!confirmPrivileged(handle, "rotate")) return;
      var res = await api("/api/tokens/rotate", "POST", { handle: handle });
      showToken(res.handle, res.token);
      notice("Rotated token for " + res.handle + ".", false);
      document.getElementById("rotate-handle").value = "";
      await refresh();
    });

    submitForm(document.getElementById("form-revoke"), async function () {
      var handle = document.getElementById("revoke-handle").value.trim();
      if (!window.confirm('Revoke token for "' + handle + '"? This is permanent.')) return;
      if (!confirmPrivileged(handle, "revoke")) return;
      await api("/api/tokens/revoke", "POST", { handle: handle });
      notice("Revoked token for " + handle + ".", false);
      document.getElementById("revoke-handle").value = "";
      await refresh();
    });

    submitForm(document.getElementById("form-channel"), async function () {
      var name = document.getElementById("channel-name").value.trim();
      var topic = document.getElementById("channel-topic").value.trim();
      await api("/api/channels", "POST", { name: name, topic: topic });
      notice("Created channel " + name + ".", false);
      document.getElementById("channel-name").value = "";
      document.getElementById("channel-topic").value = "";
      await refresh();
    });

    submitForm(document.getElementById("form-post"), async function () {
      var channel = document.getElementById("post-channel").value;
      var kind = document.getElementById("post-kind").value;
      var summary = document.getElementById("post-summary").value.trim();
      var bodyRaw = document.getElementById("post-body").value.trim();
      var mentions = document.getElementById("post-mentions").value
        .split(",")
        .map(function (s) { return s.trim(); })
        .filter(function (s) { return s.length; });
      if (kind === "task" && !mentions.length) {
        throw new Error("A task requires at least one mention (assignee).");
      }
      var payload = { channel: channel, kind: kind, summary: summary, mentions: mentions };
      if (bodyRaw) payload.body = bodyRaw;
      await api("/api/messages", "POST", payload);
      notice("Posted " + kind + " to #" + channel + ".", false);
      document.getElementById("post-summary").value = "";
      document.getElementById("post-body").value = "";
      document.getElementById("post-mentions").value = "";
      await refresh();
    });

    submitForm(document.getElementById("form-archive"), async function () {
      var channel = document.getElementById("archive-channel").value;
      if (!channel) throw new Error("Select a channel to archive.");
      if (!window.confirm('Archive channel "' + channel + '"?')) return;
      await api("/api/channels/" + encodeURIComponent(channel) + "/archive", "POST", {});
      notice("Archived channel " + channel + ".", false);
      await refresh();
    });
  }

  var conv = {
    open: null,
    oldestId: null,
    newestId: null,
    timer: null,
    seen: {}
  };

  var KIND_CLASS = {
    task: "k-task",
    decision: "k-decision",
    question: "k-question",
    answer: "k-answer"
  };

  function dmLabel(c) {
    var parts = c.members && c.members.length
      ? c.members.slice()
      : c.name.slice(3).split("+");
    if (parts.length >= 2) return parts[0] + " ↔ " + parts[1];
    return c.name.slice(3) || c.name;
  }

  function convLabel(c) {
    return c.name.indexOf("dm:") === 0 ? dmLabel(c) : c.name;
  }

  function renderConvList(list) {
    var ul = document.getElementById("conv-list");
    clear(ul);
    if (!list.length) {
      ul.appendChild(el("li", { text: "no conversations", cls: "feed-empty" }));
      return;
    }
    list.forEach(function (c) {
      var isDm = c.name.indexOf("dm:") === 0;
      var li = el("li");
      var btn = el("button", {
        cls: "conv-item" + (isDm ? " dm" : "") + (c.archived ? " archived" : "") + (conv.open === c.name ? " active" : ""),
        attrs: { type: "button" }
      });
      var top = el("div", { cls: "conv-item-top" });
      top.appendChild(el("span", { text: convLabel(c), cls: "conv-item-name" }));
      top.appendChild(el("span", { text: fmtAge(c.last_activity_ts), cls: "conv-item-age" }));
      btn.appendChild(top);
      var meta = el("div", { cls: "conv-item-meta" });
      meta.appendChild(el("span", { text: String(c.message_count) + " msg", cls: "conv-count" }));
      if (c.archived) meta.appendChild(el("span", { text: "archived", cls: "conv-item-flag" }));
      else meta.appendChild(el("span", { text: isDm ? "direct" : c.kind, cls: "conv-item-flag" }));
      btn.appendChild(meta);
      btn.addEventListener("click", function () { openConversation(c); });
      li.appendChild(btn);
      ul.appendChild(li);
    });
  }

  async function loadConversations() {
    try {
      var data = await api("/api/conversations", "GET");
      renderConvList(data.conversations || []);
    } catch (e) {
      notice("Failed to load conversations: " + e.message, true);
    }
  }

  function bubble(m) {
    var mine = m.from === "operator" || m.from === "owner";
    var node = el("div", { cls: "bubble" + (mine ? " mine" : "") });
    var meta = el("div", { cls: "bubble-meta" });
    meta.appendChild(el("span", { text: "@" + m.from, cls: "bubble-from" }));
    meta.appendChild(el("span", { text: m.kind, cls: "kind-tag " + (KIND_CLASS[m.kind] || "") }));
    meta.appendChild(el("span", { text: fmtAge(m.ts), cls: "bubble-time" }));
    node.appendChild(meta);
    node.appendChild(el("div", { text: m.summary, cls: "bubble-summary" }));
    if (m.body) node.appendChild(el("div", { text: m.body, cls: "bubble-body" }));
    if (m.to && m.to.length) {
      node.appendChild(el("div", { text: "→ " + m.to.join(", "), cls: "bubble-to" }));
    }
    if (m.resolved_at) {
      var by = m.resolved_by ? " by @" + m.resolved_by : "";
      node.appendChild(el("div", { text: "resolved" + by, cls: "bubble-resolved" }));
    }
    return node;
  }

  function olderControl() {
    var btn = el("button", { text: "load older", cls: "conv-older", attrs: { type: "button" } });
    btn.addEventListener("click", loadOlder);
    return btn;
  }

  function trackBounds(messages) {
    messages.forEach(function (m) {
      conv.seen[m.id] = true;
      if (conv.oldestId == null || m.id < conv.oldestId) conv.oldestId = m.id;
      if (conv.newestId == null || m.id > conv.newestId) conv.newestId = m.id;
    });
  }

  function channelQuery(name, before, limit) {
    var q = "/api/history?channel=" + encodeURIComponent(name) + "&limit=" + limit;
    if (before != null) q += "&before=" + before;
    return q;
  }

  async function loadOlder() {
    if (!conv.open || conv.oldestId == null) return;
    var thread = document.getElementById("conv-thread");
    try {
      var data = await api(channelQuery(conv.open, conv.oldestId, 50), "GET");
      var messages = data.messages || [];
      if (!messages.length) return;
      var anchor = thread.querySelector(".conv-older");
      var prevHeight = thread.scrollHeight;
      var frag = document.createDocumentFragment();
      messages.forEach(function (m) { frag.appendChild(bubble(m)); });
      if (anchor) thread.insertBefore(frag, anchor.nextSibling);
      else thread.insertBefore(frag, thread.firstChild);
      trackBounds(messages);
      if (messages.length >= 50 && !thread.querySelector(".conv-older")) {
        thread.insertBefore(olderControl(), thread.firstChild);
      }
      thread.scrollTop = thread.scrollHeight - prevHeight;
    } catch (e) {
      notice("Failed to load older messages: " + e.message, true);
    }
  }

  function renderThread(messages) {
    var thread = document.getElementById("conv-thread");
    clear(thread);
    if (messages.length >= 50) thread.appendChild(olderControl());
    messages.forEach(function (m) { thread.appendChild(bubble(m)); });
    if (!messages.length) {
      thread.appendChild(el("p", { text: "no messages yet — be the first.", cls: "conv-blank" }));
    }
    thread.scrollTop = thread.scrollHeight;
  }

  async function pollOpen() {
    if (!conv.open) return;
    try {
      var data = await api(channelQuery(conv.open, null, 50), "GET");
      var fresh = (data.messages || []).filter(function (m) { return !conv.seen[m.id]; });
      if (!fresh.length) return;
      var thread = document.getElementById("conv-thread");
      var atBottom = thread.scrollHeight - thread.scrollTop - thread.clientHeight < 40;
      fresh.forEach(function (m) { thread.appendChild(bubble(m)); });
      trackBounds(fresh);
      if (atBottom) thread.scrollTop = thread.scrollHeight;
    } catch (e) {
      void e;
    }
  }

  function startPoll() {
    stopPoll();
    conv.timer = setInterval(pollOpen, 4000);
  }

  function stopPoll() {
    if (conv.timer) {
      clearInterval(conv.timer);
      conv.timer = null;
    }
  }

  function highlightActive() {
    var items = document.querySelectorAll(".conv-item");
    Array.prototype.forEach.call(items, function (it) { it.classList.remove("active"); });
  }

  async function openConversation(c) {
    stopPoll();
    conv.open = c.name;
    conv.oldestId = null;
    conv.newestId = null;
    conv.seen = {};
    highlightActive();
    document.getElementById("conv-pane-title").textContent = convLabel(c);
    document.getElementById("conv-pane-sub").textContent = c.archived ? "archived" : (c.topic || "");
    var composer = document.getElementById("conv-composer");
    composer.hidden = false;
    var isDm = c.name.indexOf("dm:") === 0;
    var noteEl = document.getElementById("conv-composer-note");
    noteEl.hidden = !isDm;
    if (isDm) noteEl.textContent = "Posting here adds @operator as a participant in this DM.";
    document.getElementById("conv-error").hidden = true;
    var thread = document.getElementById("conv-thread");
    clear(thread);
    thread.appendChild(el("p", { text: "loading…", cls: "conv-blank" }));
    await loadConversations();
    try {
      var data = await api(channelQuery(c.name, null, 50), "GET");
      var messages = data.messages || [];
      renderThread(messages);
      trackBounds(messages);
      startPoll();
    } catch (e) {
      clear(thread);
      thread.appendChild(el("p", { text: "Failed to load thread: " + e.message, cls: "conv-blank" }));
    }
  }

  function wireComposer() {
    var form = document.getElementById("conv-composer");
    form.addEventListener("submit", async function (ev) {
      ev.preventDefault();
      var errEl = document.getElementById("conv-error");
      errEl.hidden = true;
      if (!conv.open) return;
      var kind = document.getElementById("conv-kind").value;
      var summary = document.getElementById("conv-summary").value.trim();
      var bodyRaw = document.getElementById("conv-body").value.trim();
      var mentions = document.getElementById("conv-mentions").value
        .split(",")
        .map(function (s) { return s.trim(); })
        .filter(function (s) { return s.length; });
      var btn = form.querySelector('button[type="submit"]');
      try {
        if (!summary) throw new Error("A summary is required.");
        if (kind === "task" && !mentions.length) {
          throw new Error("A task requires at least one mention (assignee).");
        }
        if (btn) btn.disabled = true;
        var payload = { channel: conv.open, kind: kind, summary: summary, mentions: mentions };
        if (bodyRaw) payload.body = bodyRaw;
        await api("/api/messages", "POST", payload);
        document.getElementById("conv-summary").value = "";
        document.getElementById("conv-body").value = "";
        document.getElementById("conv-mentions").value = "";
        await pollOpen();
        var thread = document.getElementById("conv-thread");
        thread.scrollTop = thread.scrollHeight;
      } catch (e) {
        errEl.textContent = e.message;
        errEl.hidden = false;
      } finally {
        if (btn) btn.disabled = false;
      }
    });
  }

  function init() {
    document.getElementById("owner-login").textContent = "owner";
    bindTokenReveal();
    wireForms();
    wireComposer();
    loadConversations();
    refresh();
    setInterval(refresh, 30000);
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", init);
  } else {
    init();
  }
})();
