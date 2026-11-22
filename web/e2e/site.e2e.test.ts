import { execFileSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { chromium, type Browser, type Page } from 'playwright'
import { createServer, type ViteDevServer } from 'vite'
import { afterAll, beforeAll, describe, expect, test } from 'vitest'

const root = fileURLToPath(new URL('..', import.meta.url))
let server: ViteDevServer
let browser: Browser
let base = ''

beforeAll(async () => {
  // The home page embeds the desktop page's demo as its app preview.
  execFileSync('pnpm', ['run', 'build:preview'], { cwd: root, stdio: 'ignore' })
  server = await createServer({ root, configFile: `${root}/vite.config.ts`, logLevel: 'error', server: { port: 0, host: '127.0.0.1' } })
  await server.listen()
  const address = server.httpServer?.address()
  if (!address || typeof address === 'string') throw new Error('Vite did not bind a TCP port')
  base = `http://127.0.0.1:${address.port}`
  browser = await chromium.launch()
})
afterAll(async () => {
  await browser?.close()
  await server?.close()
})

async function open(path: string): Promise<{ page: Page; errors: string[]; failed: string[] }> {
  const page = await browser.newPage({ locale: 'en-US' })
  const errors: string[] = []
  const failed: string[] = []
  page.on('pageerror', (error) => errors.push(String(error)))
  page.on('response', (response) => { if (response.status() >= 400) failed.push(`${response.status()} ${response.url()}`) })
  await page.goto(base + path, { waitUntil: 'load' })
  return { page, errors, failed }
}

describe('website', () => {
  test('the home page renders its hero, download link and the live app preview', async () => {
    const { page, errors, failed } = await open('/')
    await expect.poll(() => page.locator('h1').first().innerText()).toMatch(/Your\s+AI\./)
    expect(await page.title()).toContain('Beans')
    expect(await page.locator('a[href*="download"]').count()).toBeGreaterThan(0)
    const preview = page.frameLocator('iframe[src*="app-preview"]')
    await expect.poll(async () => (await preview.locator('body').innerText().catch(() => '')).includes('Launch room'), { timeout: 30_000 }).toBe(true)
    expect(errors).toEqual([])
    expect(failed).toEqual([])
    await page.close()
  })

  test('the docs render, link between pages and include the Attention page', async () => {
    const { page, errors, failed } = await open('/docs')
    await expect.poll(() => page.locator('h1').first().innerText()).toBe('Overview')
    await page.goto(`${base}/docs/attention`, { waitUntil: 'load' })
    await expect.poll(() => page.locator('h1').first().innerText()).toBe('Attention')
    expect(await page.title()).toContain('Beans Docs')
    // The sidebar links every page; following one loads it.
    const link = page.locator('a[href="/docs"]').first()
    await link.click()
    await expect.poll(() => page.locator('h1').first().innerText()).toBe('Overview')
    expect(errors).toEqual([])
    expect(failed).toEqual([])
    await page.close()
  })

  test('the Chinese site is served under /zh', async () => {
    const { page, errors } = await open('/zh')
    await expect.poll(() => page.innerText('body')).toContain('AI 队友')
    expect(await page.locator('html').getAttribute('lang')).toMatch(/^zh/)
    expect(errors).toEqual([])
    await page.close()
  })

  test('the download page offers the platforms', async () => {
    const { page, errors } = await open('/download')
    await expect.poll(() => page.locator('h1').first().innerText()).toBe('Download Beans')
    const text = await page.innerText('body')
    for (const platform of ['macOS', 'Windows', 'Linux']) expect(text).toContain(platform)
    expect(errors).toEqual([])
    await page.close()
  })

  test('install scripts, llms.txt and the served catalogs answer with the right types', async () => {
    const get = (path: string) => fetch(base + path)
    const shell = await get('/install-cli.sh')
    expect(shell.status).toBe(200)
    const script = await shell.text()
    expect(script.startsWith('#!')).toBe(true)
    expect(script).toContain('beans-v')
    expect(script).toMatch(/sha256/i)
    const powershell = await get('/install-cli.ps1')
    expect(powershell.status).toBe(200)
    expect(await powershell.text()).toMatch(/beans-v/)
    const llms = await get('/llms.txt')
    expect(llms.headers.get('content-type')).toContain('text/plain')
    expect(await llms.text()).toContain('/docs/')
    for (const path of ['/models/v1.json', '/marketplace/v1.json']) {
      const response = await get(path)
      expect(response.status, path).toBe(200)
      expect(typeof (await response.json())).toBe('object')
    }
  })

  test('an unknown page answers not found', async () => {
    const { page } = await open('/docs/no-such-page-e2e')
    const response = await fetch(`${base}/docs/no-such-page-e2e`)
    expect(response.status).toBe(404)
    await page.close()
  })
})
