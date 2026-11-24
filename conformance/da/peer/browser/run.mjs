import { build } from 'esbuild'
import { chromium } from 'playwright-core'
import { fileURLToPath } from 'node:url'
import { createServer } from 'node:http'

const [address, chain, request] = process.argv.slice(2)
const output = await build({
  entryPoints: [fileURLToPath(new URL('./client.js', import.meta.url))],
  platform: 'browser',
  bundle: true,
  write: false
})
const browser = await chromium.launch({
  executablePath: process.env.CHROME_BIN || '/usr/bin/google-chrome',
  // The private bridge is reachable only via loopback.
  args: ['--allow-loopback-in-peer-connection'],
  headless: true
})
const origin = createServer((_, response) => {
  response.writeHead(200, { 'Content-Type': 'text/html' })
  response.end('<title>DA peer fixture</title>')
})
try {
  await new Promise((resolve, reject) => {
    origin.once('error', reject)
    origin.listen(0, '127.0.0.1', resolve)
  })
  const page = await browser.newPage()
  await page.goto(`http://127.0.0.1:${origin.address().port}/`)
  await page.addScriptTag({ content: output.outputFiles[0].text })
  const response = await page.evaluate(input => window.readHeader(input), {
    address, chain, request: Array.from(Buffer.from(request, 'hex'))
  })
  process.stdout.write(Buffer.from(response))
} finally {
  await browser.close()
  origin.close()
}
