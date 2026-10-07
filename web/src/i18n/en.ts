export const en = {
  motion: { pause: 'Pause motion', resume: 'Resume motion' },
  meta: {
    title: 'Beans | AI teammates on your computer',
    description:
      'AI teammates that run on your own computer. Chat with one or several at once, let them work with your files, and they coordinate the work among themselves.',
  },
  nav: {
    turns: 'Group chats',
    relay: 'Privacy',
    tools: 'What it does',
    faq: 'FAQ',
    docs: 'Docs',
    download: 'Download',
  },
  hero: {
    badge: 'Personal AI, across your devices',
    title: '<lead>Your AI.</lead><br/>Your <accent>space.</accent>',
    accessibleTitle: 'Your AI. Your space.',
    body: 'Turn a task into a team. Give your bots files to work on, tools to connect, and a shared chat. Your computer runs the work.',
    how: 'See how it works',
    platforms: 'Mac, Windows, and Linux. Keep chatting from your phone.',
  },
  turns: {
    eyebrow: 'Group chats',
    title: 'One task. A whole team.',
    body: 'Bring your researcher, developer, and project manager into one conversation. Give each bot a role, mention the one you need, and let the others contribute when they have something useful to add.',
    alt: 'Beans on macOS: a Researcher, Developer, and Project Manager prepare a launch together in a group chat across two Runners.',
  },
  examples: {
    title: 'What will you work on?',
    body: 'Start with a request. Give your bots a role and a folder, then keep the research, decisions, and changes in one conversation.',
    choose: 'Choose an example workflow',
    caption: 'Illustrative workflows. Outputs depend on your provider, connected tools, and instructions.',
    items: [
      { name: 'Research', prompt: 'Compare the options and write a recommendation with sources.', steps: ['Search and read the sources', 'Compare the approaches', 'Write a recommendation'] },
      { name: 'Build', prompt: 'Review this project and help me implement the next feature.', steps: ['Read the project files', 'Edit and run commands', 'Review the changes together'] },
      { name: 'Write', prompt: 'Turn these notes into a clear project brief.', steps: ['Read your notes', 'Draft the brief', 'Refine it in the same chat'] },
    ],
  },
  relay: {
    eyebrow: 'Privacy',
    title: 'Your work stays with you.',
    body: 'Your computer runs the bots. Your chosen AI provider generates their replies. Pair your phone to take the conversation with you, with end-to-end encrypted sync between your devices.',
    alt: 'Encrypted data goes from your computer through the relay to your phone. Only your devices can decrypt it.',
    caption: 'Illustration of encrypted device sync.',
    states: { encrypt: 'Encrypting', forward: 'Forwarding', decrypt: 'Decrypting' },
    nodes: {
      computer: { title: 'Your computer', body: 'Encrypted before sending' },
      relay: { title: 'Relay', body: "Forwards encrypted data. Can't decrypt it." },
      phone: { title: 'Your phone', body: 'Decrypted with your key' },
    },
  },
  tools: {
    eyebrow: 'Tools',
    title: 'Files, commands, and the web.',
    body: 'Go beyond answers. Give a bot a working folder, connect the tools you use, and let it research, write, and make changes you can inspect.',
    kinds: {
      files: { title: 'Files', body: 'Opens, edits, and creates files in the folder you give it, and runs commands there.' },
      web: { title: 'Web', body: 'Searches the web and reads pages.' },
      memory: { title: 'Memory', body: "Keeps notes across chats, so you don't have to repeat yourself." },
      plugins: { title: 'Apps', body: 'Connects to GitHub, Notion, Linear, and other apps. Asks for permission first.' },
    },
  },
  chef: {
    eyebrow: 'Getting started',
    title: 'Meet your first teammate.',
    body: 'Every new account starts with one bot, Chef. Tell Chef what you work on and it suggests a few bots, each for one kind of task. Approve them and Chef creates them. You can rename or delete any bot later.',
    steps: [
      { title: 'Create your account', body: 'No email, no password. You get a backup phrase to write down.' },
      { title: 'Connect an AI provider', body: 'Sign in with ChatGPT or Grok, or paste a DeepSeek API key.' },
      { title: 'Talk to Chef', body: 'Tell it what you work on and approve the bots it suggests.' },
      { title: 'Add another computer', body: 'Paste a pairing code, then choose which bots run on it.' },
    ],
  },
  faq: {
    title: 'Questions',
    items: [
      { q: 'Do I need an account or a server?', a: 'Your backup phrase identifies your account. Bots run on your computer. Pairing and sync use a relay you choose or host.' },
      { q: 'What does it run on?', a: 'Mac, Windows, and Linux, with the app or only the command line. iPhone and iPad are in beta, for chatting with your bots. A bot on any of your computers can join the same group chats.' },
      { q: 'Which AI does it use?', a: 'Your own account or API key. Sign in with ChatGPT or Grok, or add a DeepSeek API key. Each bot can use a different provider, and you can change it any time.' },
      { q: 'What can a bot do on my computer?', a: 'Read, edit, and create files and run commands in the folder you give it. It runs with your user permissions on that computer and shows you what it ran.' },
      { q: 'Can anyone read my chats?', a: 'Your paired devices can read your chats. The relay handles encrypted data without its keys. The AI provider you choose processes the content sent to it to generate replies.' },
    ],
  },
  cta: {
    title: 'Give your next task a team.',
    body: 'Download Beans, connect your AI provider, and tell Chef what you want to work on.',
  },
  docs: {
    title: 'Beans Docs',
  },
  download: {
    title: 'Download Beans',
    description: 'Check client download availability for Mac, Windows, Linux, Android and iOS, or install the CLI.',
    version: 'Version {{version}}',
    unavailable: 'No published download is available for this platform yet.',
    loadFailed: "Couldn't load the latest version. Reload the page to try again.",
    mac: {
      title: 'Mac',
      body: 'Chat with your bots and run them on your Mac.',
      action: 'Download for Mac',
      system: 'macOS {{version}} or later',
      appleSilicon: 'Apple silicon',
    },
    windows: {
      title: 'Windows',
      body: 'Chat with your bots and run them on your Windows PC. Bots run their commands in the bash that comes with <git>Git for Windows</git>, so install it too.',
      action: 'Download for Windows',
      system: 'x64',
    },
    linux: {
      title: 'Linux',
      body: 'Chat with your bots and run them on your Linux computer. The command installs Beans in your home folder, without root, and Beans keeps itself up to date.',
      action: 'Download for Linux',
      deb: 'Or install the Debian package for <amd64>x64</amd64> or <arm64>Arm64</arm64>. It updates when you install the next one.',
      system: 'x64 and Arm64',
    },
    ios: {
      title: 'iPhone and iPad',
      body: 'Chat with your bots while they keep running on your computer. Scan the QR code in the desktop app to pair.',
      action: 'Download IPA',
      note: 'Ad-hoc IPAs install only on devices registered in the signing profile. iOS installation approval is required.',
    },
    android: {
      title: 'Android',
      body: 'Chat with your bots from your Android phone.',
      action: 'Download APK',
    },
    cli: {
      title: 'Beans CLI',
      body: 'Makes a computer a Runner: it runs your bots, with no chat window. The Mac, Windows, and Linux apps have it built in, so you need it only on a computer without the app. Pair it with your account, then create bots for it from the desktop app or your phone.',
      unix: 'macOS and Linux',
      windows: 'Windows (PowerShell)',
      note: '<docs>CLI docs</docs>',
      copy: 'Copy',
      copied: 'Copied',
    },
  },
  footer: {
    privacy: 'Privacy',
    support: 'Support',
    faq: 'FAQ',
    download: 'Download',
  },
}

export type Messages = typeof en
