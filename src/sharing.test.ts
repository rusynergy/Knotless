import { describe, expect, it } from 'vitest';
import QRCode from 'qrcode';
import jsQR from 'jsqr';
import { parseVless } from './domain';
import { serverLink } from './sharing';

const id = '11111111-1111-4111-8111-111111111111';
describe('server sharing', () => {
  it.each([
    'type=tcp&security=none',
    'type=raw&security=reality&sni=example.com&pbk=test-public-key&sid=abcd&spx=%2Fhello%3Fx%3D1&fp=chrome&flow=xtls-rprx-vision',
    'type=ws&security=tls&sni=example.com&host=cdn.example.com&path=%2Fws%3Fed%3D2048&alpn=h2%2Chttp%2F1.1&fp=firefox',
    'type=grpc&security=tls&serviceName=a%2Fb&mode=multi',
    'type=xhttp&security=reality&sni=example.com&pbk=public-key&host=cdn.example.com&path=%2Fapi&mode=packet-up',
    'type=httpupgrade&security=tls&host=cdn.example.com&path=%2Fupgrade',
  ])('preserves connection settings: %s', params => {
    const original = parseVless(`vless://${id}@[2001:db8::1]:8443?${params}#${encodeURIComponent('Москва / 北京 & demo')}`);
    original.subscriptionId = 'private-subscription-id';
    const link = serverLink(original);
    const restored = parseVless(link);
    expect(restored.outbound).toEqual(original.outbound);
    expect(restored.name).toBe(original.name);
    expect(link).not.toContain(original.subscriptionId);
  });
  it('decodes the generated QR back into the same working profile', () => {
    const server = parseVless(`vless://${id}@example.com:443?type=ws&security=tls&path=%2Fa%3Fb%3Dc#${encodeURIComponent('Тест 中文')}`);
    const link = serverLink(server);
    const qr = QRCode.create(link, { errorCorrectionLevel: 'M' });
    const scale = 6, margin = 4, size = (qr.modules.size + margin * 2) * scale;
    const rgba = new Uint8ClampedArray(size * size * 4).fill(255);
    for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) {
      const row = Math.floor(y / scale) - margin, col = Math.floor(x / scale) - margin;
      if (row >= 0 && col >= 0 && row < qr.modules.size && col < qr.modules.size && qr.modules.get(row, col)) {
        const offset = (y * size + x) * 4; rgba[offset] = rgba[offset + 1] = rgba[offset + 2] = 0;
      }
    }
    const result = jsQR(rgba, size, size);
    expect(result?.data).toBe(link);
    expect(parseVless(result!.data).outbound).toEqual(server.outbound);
  });
});
