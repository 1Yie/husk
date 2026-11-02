/** The application's global shortcut listener.
 *
 * One capture-phase listener resolves a key event to a shortcut action and runs
 * the command registered for it. Focus decides what an action may touch:
 * a terminal keeps its own keys except the terminal toggle, a text field keeps
 * the keys a workspace action would steal, and an open overlay pauses the
 * actions that would change what is behind it.
 */
import { useEffect } from 'react';
import { useLayoutStore } from '@/app/layout/layout-store';
import { useDockStore } from '@/features/chat/components/dock/state';
import { registerCommand, runCommand } from '@/lib/command-registry';
import { openSettings } from '@/lib/settings-bus';
import { actionForEvent, shortcutDefinition } from '@/lib/shortcuts';
import { useShortcutStore } from '@/lib/shortcuts-store';

export function isTerminalFocused(): boolean {
	const active = document.activeElement;
	return active instanceof Element && active.closest('.xterm') !== null;
}

function isTextEntry(element: Element | null): boolean {
	if (!(element instanceof HTMLElement)) return false;
	if (element.isContentEditable) return true;
	return element.tagName === 'INPUT' || element.tagName === 'TEXTAREA';
}

function isOverlayOpen(): boolean {
	if (document.querySelector('.settings-layout')) return true;
	return document.querySelector('[role="dialog"][data-state="open"]') !== null;
}

/** Whether the focused element claims this action's keys. */
export function shortcutAllowed(
	action: Parameters<typeof shortcutDefinition>[0],
	active = document.activeElement
): boolean {
	if (isTerminalFocused()) return action === 'terminal';
	const definition = shortcutDefinition(action);
	if (
		definition.scope === 'workspace' &&
		(isTextEntry(active) || isOverlayOpen())
	) {
		return false;
	}
	return true;
}

/** Store-backed commands; the shell mounts this once. */
export function registerShellCommands(): () => void {
	const disposers = [
		registerCommand('terminal', () =>
			useLayoutStore.getState().toggleTerminal()
		),
		registerCommand('session-sidebar', () =>
			useLayoutStore.getState().toggleSidebar()
		),
		registerCommand('tools-sidebar', () => useDockStore.getState().toggle()),
		registerCommand('settings', () => openSettings('general')),
	];
	return () => {
		for (const dispose of disposers) dispose();
	};
}

export function useAppShortcuts() {
	useEffect(registerShellCommands, []);
	useEffect(() => {
		const onKeyDown = (event: KeyboardEvent) => {
			if (event.defaultPrevented) return;
			const action = actionForEvent(
				event,
				useShortcutStore.getState().bindings
			);
			if (!action || !shortcutAllowed(action)) return;
			if (!runCommand(action)) return;
			event.preventDefault();
			event.stopPropagation();
		};
		window.addEventListener('keydown', onKeyDown, true);
		return () => window.removeEventListener('keydown', onKeyDown, true);
	}, []);
}
