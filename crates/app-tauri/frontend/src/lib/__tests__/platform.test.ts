import { afterEach, expect, test, vi } from 'vitest';
import { shortcutLabel } from '../platform';

test('shortcuts are spelled per platform, never both at once', () => {
	expect(shortcutLabel(['mod', 'shift', 'j'], true)).toBe('⌘⇧J');
	expect(shortcutLabel(['mod', 'shift', 'j'], false)).toBe('Ctrl+Shift+J');
	expect(shortcutLabel(['mod', 'c'], true)).toBe('⌘C');
	expect(shortcutLabel(['mod', 'alt', 'Enter'], false)).toBe('Ctrl+Alt+Enter');
});

const MAC =
	'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15';
const WINDOWS = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36';
const LINUX = 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36';

async function load(userAgent: string, platform: string) {
	vi.resetModules();
	vi.stubGlobal('navigator', { userAgent, platform });
	return import('../platform');
}

afterEach(() => {
	vi.unstubAllGlobals();
	vi.resetModules();
});

test('the framed shell belongs to macOS and Windows only', async () => {
	expect((await load(MAC, 'MacIntel')).framedShell).toBe(true);
	expect((await load(WINDOWS, 'Win32')).framedShell).toBe(true);
	const linux = await load(LINUX, 'Linux x86_64');
	expect(linux.isLinux).toBe(true);
	expect(linux.framedShell).toBe(false);
});
