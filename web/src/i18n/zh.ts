import type { Messages } from './en'

export const zh: Messages = {
  motion: { pause: '暂停动画', resume: '继续动画' },
  meta: {
    title: 'Beans | 在你的电脑上运行的 AI 队友',
    description:
      '运行在你自己电脑上的 AI 队友。可以单聊，也可以拉群，让它们处理你的文件和任务，它们会自己分工协作。',
  },
  nav: {
    language: '语言',
    costs: '费用',
    compare: '比较',
    turns: '群聊',
    relay: '隐私',
    tools: '它能做什么',
    faq: '常见问题',
    docs: '文档',
    download: '下载',
  },
  hero: {
    badge: 'Windows 和 Linux 版现已推出',
    title: '<lead>拉个群，</lead><br/>让 AI 队友<accent>分工</accent>。',
    accessibleTitle: '拉个群，让 AI 队友分工。',
    body: '和一个队友单聊，或者把几个拉进一个群聊。它们可以读写你的文件、运行命令，并记住你说过的话。交给它们一件事，它们会自己分工协作。',
    how: '看看它怎么运作',
    platforms: '支持 Mac、Windows 和 Linux，iPhone 和 iPad 版正在公测。',
  },
  turns: {
    eyebrow: '群聊',
    title: '一个任务，一整个团队。',
    body: '把研究员、开发者和项目经理放进同一个对话。为每个智能体分配角色，@ 你需要的队友，其他队友有有用的信息时再参与。',
    alt: 'macOS 上的 Beans：研究员、开发者和项目经理分别在两台运行设备上工作，在群聊中一起准备产品发布。',
  },
  relay: {
    eyebrow: '隐私',
    title: '工作由你掌控。',
    body: '你的电脑运行智能体，你选择的 AI 服务商生成回复。配对手机，随时继续对话，设备间通过端到端加密同步。',
    alt: '加密数据从你的电脑经过中继传到你的手机，只有你的设备能解密。',
    caption: '设备间加密同步示意。',
    states: { encrypt: '加密中', forward: '转发中', decrypt: '解密中' },
    nodes: {
      computer: { title: '你的电脑', body: '发送前已加密' },
      relay: { title: '中继', body: '只负责传输，无法解密' },
      phone: { title: '你的手机', body: '用你的密钥解密' },
    },
  },
  tools: {
    eyebrow: '工具',
    title: '文件、命令和网页。',
    body: '在你的任意一台电脑上给智能体一个文件夹。它只在那个文件夹里、那台电脑上工作，并告诉你它运行了什么。',
    kinds: {
      files: {
        title: '文件',
        body: '在你给它的文件夹里打开、修改和新建文件，也在那里运行命令。',
      },
      web: { title: '网络', body: '搜索网页并阅读页面内容。' },
      memory: {
        title: '记忆',
        body: '跨聊天保留笔记，同一件事不用再说第二遍。',
      },
      plugins: {
        title: '应用',
        body: '接入 GitHub、Notion、Linear 等应用。操作前会先征求你的同意。',
      },
    },
  },
  chef: {
    eyebrow: '开始使用',
    title: '从一个智能体开始。',
    body: '每个新账户都自带一个名叫“幕僚长”的智能体。告诉它你平时做什么，它会建议几个智能体，各负责一类任务。你确认后，幕僚长就把它们创建出来。之后随时可以改名或删除。',
    steps: [
      {
        title: '创建账户',
        body: '不用邮箱，不用密码，只有一串要抄下来的备份短语。',
      },
      {
        title: '连接 AI 服务商',
        body: '登录 ChatGPT 或 Grok，或者粘贴一个 DeepSeek API 密钥。',
      },
      {
        title: '和幕僚长聊聊',
        body: '说说你平时做什么，然后确认它建议的智能体。',
      },
      {
        title: '再加一台电脑',
        body: '粘贴配对码，再选择哪些智能体在那台电脑上运行。',
      },
    ],
  },
  faq: {
    title: '常见问题',
    items: [
      {
        q: 'Beans 免费吗？',
        a: '是的，Beans 费用为 $0。订阅或 API 使用费用直接付给 AI 提供商，提供商的用量限制和费率另行适用。',
      },
      {
        q: '需要注册账号或者服务器吗？',
        a: '备份短语用于识别你的账户。智能体在你的电脑上运行，设备配对和同步使用你选择或自行部署的中继。',
      },
      {
        q: '支持哪些平台？',
        a: 'Mac、Windows 和 Linux，用应用或只用命令行都可以。iPhone 和 iPad 版正在公测，用来和智能体聊天。你任何一台电脑上的智能体都能加入同一个群聊。',
      },
      {
        q: '它用的是哪家 AI？',
        a: '你自己的账号或 API 密钥。登录 ChatGPT 或 Grok，或者添加一个 DeepSeek API 密钥。每个智能体可以用不同的服务商，随时可以换。',
      },
      {
        q: '智能体能在我的电脑上做什么？',
        a: '在你给它的文件夹里读取、修改、新建文件和运行命令。它以你的用户权限在那台电脑上运行，并告诉你它运行了什么。',
      },
      {
        q: '有人能看到我的聊天记录吗？',
        a: '你配对的设备可以读取聊天。中继处理加密数据，但没有解密密钥。你选择的 AI 服务商会处理发送给它的内容，以生成回复。',
      },
    ],
  },
  cta: {
    title: '把下一个任务交给团队。',
    body: '下载 Beans，连接 AI 服务商，告诉幕僚长你想完成什么工作。',
  },
  docs: {
    title: 'Beans 文档',
  },
  download: {
    title: '下载 Beans',
    description: '查看 Mac、Windows、Linux、Android 和 iOS 客户端的下载可用情况，或安装 CLI。',
    version: '版本 {{version}}',
    unavailable: '此平台尚无已发布的下载文件。',
    loadFailed: '暂时无法获取最新版本，请刷新页面重试。',
    mac: {
      title: 'Mac',
      body: '和智能体聊天，并在你的 Mac 上运行它们。',
      action: '下载 Mac 版',
      system: 'macOS {{version}} 或更高版本',
      appleSilicon: 'Apple 芯片',
    },
    windows: {
      title: 'Windows',
      body: '和智能体聊天，并在你的 Windows 电脑上运行它们。智能体用 <git>Git for Windows</git> 自带的 bash 运行命令，请一并安装。',
      action: '下载 Windows 版',
      system: 'x64',
    },
    linux: {
      title: 'Linux',
      body: '和智能体聊天，并在你的 Linux 电脑上运行它们。这条命令把 Beans 安装到你的主目录，不需要 root，之后 Beans 会自动更新。',
      action: '下载 Linux 版',
      deb: '也可以安装 <amd64>x64</amd64> 或 <arm64>Arm64</arm64> 的 Debian 软件包，它在你安装下一个软件包时更新。',
      system: 'x64 和 Arm64',
    },
    ios: {
      title: 'iPhone 和 iPad',
      body: '和智能体聊天，它们继续在你的电脑上运行。扫描桌面应用里的二维码即可配对。',
      action: '下载 IPA',
      note: 'Ad-hoc IPA 仅能安装到签名描述文件中注册的设备，并需要 iOS 安装批准。',
    },
    android: {
      title: 'Android',
      body: '在 Android 手机上和智能体聊天。',
      action: '下载 APK',
    },
    cli: {
      title: 'Beans CLI',
      body: '让一台电脑成为 Runner：它运行你的智能体，但没有聊天界面。Mac、Windows 和 Linux 应用都已内置 CLI，只有在没装应用的电脑上才需要单独安装。把它和你的账户配对，再用桌面应用或手机为它创建智能体。',
      unix: 'macOS 和 Linux',
      windows: 'Windows（PowerShell）',
      note: '<docs>CLI 文档</docs>',
      copy: '复制',
      copied: '已复制',
    },
  },
  footer: {
    privacy: '隐私',
    support: '支持',
    faq: '常见问题',
    download: '下载',
  },
}
