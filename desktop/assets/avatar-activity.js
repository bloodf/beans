(() => {
  // src/model/avatarActivity.ts
  var priority = ["error", "retry", "waiting", "working", "responding", "thinking", "idle"];
  function avatarActivity(botID, input, now = Date.now()) {
    if (input.errorUntil !== undefined && now < input.errorUntil)
      return "error";
    if (!input.active)
      return "idle";
    if (input.retrying)
      return "retry";
    let start = input.startedAt ?? 0;
    for (const message of input.messages)
      if (message.author.kind === "you" && !message.queued)
        start = Math.max(start, message.createdAt);
    let result = input.thinking ? "thinking" : "idle";
    for (const message of input.messages) {
      if (message.createdAt < start || message.author.kind !== "bot" || message.author.botID !== botID)
        continue;
      const body = message.body;
      let state = "idle";
      if (body.kind === "permission" && body.request.decision === "pending")
        state = "waiting";
      else if (body.kind === "tool" && body.tool.run?.state === "asking")
        state = "waiting";
      else if (body.kind === "tool" && body.tool.isRunning)
        state = body.tool.run?.state === "waiting" ? "waiting" : "working";
      else if (body.kind === "text" && message.state.kind === "streaming")
        state = "responding";
      else if (message.state.kind === "thinking")
        state = "thinking";
      if (priority.indexOf(state) < priority.indexOf(result))
        result = state;
    }
    return result;
  }
  function aggregateAvatarActivity(states) {
    return priority.find((state) => states.includes(state)) ?? "idle";
  }

  // src/model/nativeAvatarActivity.ts
  function toMessage(wire) {
    const body = wire.body;
    return {
      id: wire.id,
      author: wire.author.kind === "bot" ? { kind: "bot", botID: wire.author.bot_id } : { kind: wire.author.kind },
      body: body.kind === "permission" ? { kind: "permission", request: { decision: body.decision ?? "pending" } } : body.kind === "tool" ? { kind: "tool", tool: { isRunning: body.is_running ?? false, run: body.run ? { state: body.run.state } : undefined } } : { kind: body.kind, text: body.text ?? "" },
      state: wire.state,
      createdAt: wire.created_at * 1000,
      queued: wire.queued ?? undefined
    };
  }

  class NativeAvatarActivity {
    chats = new Map;
    jobs = new Map;
    retry = new Map;
    thinking = new Map;
    errors = new Map;
    reset() {
      this.chats.clear();
      this.jobs.clear();
      this.retry.clear();
      this.thinking.clear();
      this.errors.clear();
    }
    clearErrors(chat, bot) {
      for (const key of this.errors.keys())
        if (key === JSON.stringify([chat, bot]) || !bot && JSON.parse(key)[0] === chat)
          this.errors.delete(key);
    }
    event(name, data, now) {
      if (name === "snapshot") {
        this.reset();
        for (const c of data.chats ?? [])
          this.chats.set(c.id, { bots: c.bot_ids, messages: (c.messages ?? []).map(toMessage) });
        for (const j of data.running_turns ?? [])
          this.jobs.set(j.job_id, j);
      } else if (name === "roster.changed") {
        const retained = new Set;
        for (const c of data.chats) {
          retained.add(c.id);
          const old = this.chats.get(c.id);
          for (const b of old?.bots ?? [])
            if (!c.bot_ids.includes(b)) {
              this.clearErrors(c.id, b);
              for (const [id, j] of this.jobs)
                if (j.chat_id === c.id && j.bot_id === b)
                  this.jobs.delete(id);
              if (this.retry.get(c.id) === b)
                this.retry.delete(c.id);
              if (this.thinking.get(c.id) === b)
                this.thinking.delete(c.id);
            }
          this.chats.set(c.id, { bots: c.bot_ids, messages: old?.messages ?? [] });
        }
        for (const id of this.chats.keys())
          if (!retained.has(id))
            this.removeChat(id);
      } else if (name === "chat.removed")
        this.removeChat(data.chat_id);
      else if (name === "job.started") {
        this.clearErrors(data.chat_id, data.bot_id);
        if (this.retry.get(data.chat_id) === data.bot_id)
          this.retry.delete(data.chat_id);
        this.jobs.set(data.job_id, { ...data, previous: new Set(this.chats.get(data.chat_id)?.messages.map((m) => m.id)) });
      } else if (name === "job.finished") {
        this.jobs.delete(data.job_id);
        if (!data.bot_id || this.retry.get(data.chat_id) === data.bot_id)
          this.retry.delete(data.chat_id);
        if (!data.bot_id || this.thinking.get(data.chat_id) === data.bot_id)
          this.thinking.delete(data.chat_id);
      } else if (name === "job.retry")
        this.retry.set(data.chat_id, data.bot_id);
      else if (name === "job.thinking")
        this.thinking.set(data.chat_id, data.bot_id);
      else if (name === "message.removed") {
        const c = this.chats.get(data.chat_id);
        if (c)
          c.messages = c.messages.filter((m) => m.id !== data.message_id);
        this.clearErrors(data.chat_id);
      } else if (name === "message.added" || name === "message.updated") {
        const c = this.chats.get(data.chat_id);
        if (!c)
          return;
        const m = toMessage(data.message);
        if (m.author.kind === "you" && !m.queued)
          this.clearErrors(data.chat_id);
        if (m.author.kind === "bot") {
          const bot = m.author.botID;
          if (this.retry.get(data.chat_id) === bot)
            this.retry.delete(data.chat_id);
          if (name === "message.added" && this.thinking.get(data.chat_id) === bot)
            this.thinking.delete(data.chat_id);
          const current = [...this.jobs.values()].some((j) => j.chat_id === data.chat_id && j.bot_id === bot && !j.previous?.has(m.id));
          const latestUser = Math.max(0, ...c.messages.filter((v) => v.author.kind === "you" && !v.queued).map((v) => v.createdAt));
          if (current && m.createdAt >= latestUser && m.state.kind === "failed")
            this.errors.set(JSON.stringify([data.chat_id, bot]), now + 5000);
        }
        const index = c.messages.findIndex((v) => v.id === m.id);
        if (index < 0)
          c.messages.push(m);
        else
          c.messages[index] = m;
      }
    }
    removeChat(id) {
      this.chats.delete(id);
      this.retry.delete(id);
      this.thinking.delete(id);
      this.clearErrors(id);
      for (const [key, job] of this.jobs)
        if (job.chat_id === id)
          this.jobs.delete(key);
    }
    states(now) {
      for (const [key, until] of this.errors)
        if (until <= now)
          this.errors.delete(key);
      const chats = {};
      const all = new Map;
      for (const [id, chat] of this.chats) {
        chats[id] = {};
        for (const bot of chat.bots) {
          const jobs = [...this.jobs.values()].filter((j) => j.chat_id === id && j.bot_id === bot);
          const state = avatarActivity(bot, {
            active: jobs.length > 0,
            messages: chat.messages.filter((m) => jobs.some((j) => !j.previous?.has(m.id))),
            retrying: this.retry.get(id) === bot,
            thinking: this.thinking.get(id) === bot,
            errorUntil: this.errors.get(JSON.stringify([id, bot]))
          }, now);
          chats[id][bot] = state;
          const states = all.get(bot) ?? [];
          states.push(state);
          all.set(bot, states);
        }
      }
      return {
        chats,
        bots: Object.fromEntries([...all].map(([id, states]) => [id, aggregateAvatarActivity(states)])),
        expires: this.errors.size ? Math.min(...this.errors.values()) : 0
      };
    }
  }
  Object.assign(globalThis, { nativeAvatarActivity: new NativeAvatarActivity });
})();
