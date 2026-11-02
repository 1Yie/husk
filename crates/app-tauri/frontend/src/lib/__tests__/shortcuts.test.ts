import { beforeEach, describe, expect, test, vi } from 'vitest';
import { isMac } from '@/lib/platform';
import {
	actionForEvent,
	bindingFromEvent,
	bindingLabel,
	bindingOwner,
	defaultBindings,
} from '@/lib/shortcuts';
import { useShortcutStore } from '@/lib/shortcuts-store';
import { shortcutAllowed } from '@/lib/use-app-shortcuts';

const modifier = isMac ? { metaKey: true } : { ctrlKey: true };

function key(init: KeyboardEventInit) {
	return new KeyboardEvent('keydown', init);
}

beforeEach(() => {
	localStorage.clear();
	useShortcutStore.setState({ bindings: defaultBindings() });
});

describe('binding grammar', () => {
	test('modifier-only presses are not bindings', () => {
		expect(
			bindingFromEvent({
				key: 'Shift',
				ctrlKey: false,
				metaKey: false,
				shiftKey: true,
				altKey: false,
			})
		).toBeNull();
	});

	test('the platform primary modifier renders as the stored mod prefix', () => {
		expect(
			bindingFromEvent({
				key: 'J',
				shiftKey: true,
				altKey: true,
				ctrlKey: !isMac,
				metaKey: isMac,
			})
		).toBe('mod+alt+shift+j');
		expect(bindingLabel('mod+shift+j')).toBe(isMac ? '⌘⇧J' : 'Ctrl+Shift+J');
	});

	test('a pressed combination resolves to its action and ignores repeat / composition', () => {
		const bindings = defaultBindings();
		expect(
			actionForEvent(key({ key: 'j', shiftKey: true, ...modifier }), bindings)
		).toBe('terminal');
		expect(
			actionForEvent(
				key({ key: 'j', shiftKey: true, repeat: true, ...modifier }),
				bindings
			)
		).toBeNull();
		expect(
			actionForEvent(
				key({ key: 'j', shiftKey: true, isComposing: true, ...modifier }),
				bindings
			)
		).toBeNull();
		expect(actionForEvent(key({ key: 'j', ...modifier }), bindings)).toBeNull();
	});

	test('two actions cannot share one combination', () => {
		const bindings = { ...defaultBindings(), 'new-session': 'mod+shift+j' };
		expect(bindingOwner('mod+shift+j', bindings, 'terminal')).toBe(
			'new-session'
		);
		expect(bindingOwner('mod+shift+j', bindings, 'new-session')).toBe(
			'terminal'
		);
	});
});

describe('focus rules', () => {
	test('a focused terminal keeps every key except the terminal toggle', () => {
		const terminal = document.createElement('div');
		terminal.className = 'xterm';
		const area = document.createElement('textarea');
		terminal.append(area);
		document.body.append(terminal);
		area.focus();
		expect(shortcutAllowed('terminal')).toBe(true);
		expect(shortcutAllowed('new-session')).toBe(false);
		expect(shortcutAllowed('session-sidebar')).toBe(false);
		terminal.remove();
	});

	test('a text field keeps the keys a workspace action would take', () => {
		const input = document.createElement('input');
		document.body.append(input);
		input.focus();
		expect(shortcutAllowed('new-session')).toBe(false);
		expect(shortcutAllowed('mode-build')).toBe(false);
		expect(shortcutAllowed('terminal')).toBe(true);
		input.remove();
	});

	test('an open settings overlay pauses workspace actions', () => {
		const settings = document.createElement('div');
		settings.className = 'settings-layout';
		document.body.append(settings);
		expect(shortcutAllowed('workspace-file')).toBe(false);
		expect(shortcutAllowed('terminal')).toBe(true);
		settings.remove();
	});
});

describe('store', () => {
	test('a rebind persists and a failed write keeps the old binding', () => {
		expect(useShortcutStore.getState().setBinding('terminal', 'mod+k')).toBe(
			true
		);
		expect(
			JSON.parse(localStorage.getItem('husk.shortcuts.v1') ?? '{}').terminal
		).toBe('mod+k');

		const failing = vi
			.spyOn(Storage.prototype, 'setItem')
			.mockImplementation(() => {
				throw new Error('quota');
			});
		expect(useShortcutStore.getState().setBinding('terminal', 'mod+l')).toBe(
			false
		);
		expect(useShortcutStore.getState().bindings.terminal).toBe('mod+k');
		failing.mockRestore();
	});

	test('clearing leaves the action without a binding and reset restores defaults', () => {
		useShortcutStore.getState().clearBinding('settings');
		expect(useShortcutStore.getState().bindings.settings).toBe('');
		useShortcutStore.getState().resetAll();
		expect(useShortcutStore.getState().bindings).toEqual(defaultBindings());
	});
});
