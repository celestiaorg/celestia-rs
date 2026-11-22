import { noise } from '@chainsafe/libp2p-noise'
import { webTransport } from '@libp2p/webtransport'
import { webRTCDirect } from '@libp2p/webrtc'
import { multiaddr } from '@multiformats/multiaddr'
import { createLibp2p } from 'libp2p'

window.readHeader = async ({ address, chain, request }) => {
  const node = await createLibp2p({
    addresses: { listen: [] },
    transports: address.includes('/webtransport/')
      ? [webTransport()]
      : [webRTCDirect({ dialerVersion: 2, rtcConfiguration: { iceServers: [] } })],
    connectionEncrypters: [noise()],
    // This private fixture intentionally connects to a loopback peer.
    connectionGater: { denyDialMultiaddr: () => false }
  })
  let stage = 'dial header protocol'
  try {
    const signal = AbortSignal.timeout(30000)
    const stream = await node.dialProtocol(multiaddr(address), '/' + chain + '/header-ex/v0.0.3', {
      signal, onProgress: event => { stage = event.type }
    })
    signal.addEventListener('abort', () => stream.abort(signal.reason), { once: true })
    stage = 'send header request'
    stream.send(Uint8Array.from(request))
    await stream.close({ signal })
    stage = 'read signed header'
    const bytes = []
    for await (const chunk of stream) {
      if (bytes.length + chunk.byteLength > 1024 * 1024) throw new Error('oversized header response')
      for (const byte of chunk.subarray()) bytes.push(byte)
    }
    return bytes
  } catch (error) {
    throw new Error(`${stage}: ${error.message}`)
  } finally {
    await node.stop()
  }
}
