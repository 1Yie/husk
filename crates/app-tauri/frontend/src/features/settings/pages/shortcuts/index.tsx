// 快捷键设置 — one shared `ItemRow` per application action. A row captures the
// next combination in place, so rebinding never needs a dialog.
import { useEffect, useState } from 'react';
import { toast } from 'sonner';
import {
	Code,
	FileText,
	FolderOpen,
	PanelLeft,
	PanelRight,
	Plus,
	Settings,
	SquareTerminal,
} from 'lucide-react';
import { Button } from '@/components/ui/button';
import { KvList, KvListContent } from '@/components/ui/kv-list';
import { ItemIcon, ItemRow } from '@/features/settings/components/item-list';
import { SettingsSection } from '@/features/settings/components';
import {
	bindingFromEvent,
	bindingLabel,
	bindingOwner,
	SHORTCUTS,
	shortcutDefinition,
	type ShortcutAction,
} from '@/lib/shortcuts';
import { useShortcutStore } from '@/lib/shortcuts-store';

const ACTION_ICONS: Record<ShortcutAction, React.ReactNode> = {
	terminal: <SquareTerminal className="h-4 w-4" />,
	'session-sidebar': <PanelLeft className="h-4 w-4" />,
	'tools-sidebar': <PanelRight className="h-4 w-4" />,
	settings: <Settings className="h-4 w-4" />,
	'new-session': <Plus className="h-4 w-4" />,
	'workspace-file': <FolderOpen className="h-4 w-4" />,
	'mode-build': <Code className="h-4 w-4" />,
	'mode-office': <FileText className="h-4 w-4" />,
};

export function ShortcutsSettings() {
	const bindings = useShortcutStore((state) => state.bindings);
	const setBinding = useShortcutStore((state) => state.setBinding);
	const clearBinding = useShortcutStore((state) => state.clearBinding);
	const resetAll = useShortcutStore((state) => state.resetAll);
	const [capturing, setCapturing] = useState<ShortcutAction | null>(null);

	useEffect(() => {
		if (!capturing) return;
		const onKeyDown = (event: KeyboardEvent) => {
			event.preventDefault();
			event.stopPropagation();
			if (event.key === 'Escape') {
				setCapturing(null);
				return;
			}
			const binding = bindingFromEvent(event);
			if (!binding) return;
			const owner = bindingOwner(binding, bindings, capturing);
			if (owner) {
				toast.error(
					`${bindingLabel(binding)} 已被「${shortcutDefinition(owner).label}」使用`
				);
				return;
			}
			if (!setBinding(capturing, binding))
				toast.error('保存失败，已保留原快捷键');
			setCapturing(null);
		};
		window.addEventListener('keydown', onKeyDown, true);
		return () => window.removeEventListener('keydown', onKeyDown, true);
	}, [capturing, bindings, setBinding]);

	return (
		<div className="flex w-full min-w-0 flex-col gap-8">
			<SettingsSection
				title="应用操作"
				actions={
					<Button
						type="button"
						size="sm"
						variant="outline"
						onClick={() => {
							if (!resetAll()) toast.error('保存失败，已保留原快捷键');
						}}
					>
						恢复默认
					</Button>
				}
			>
				<KvList>
					<KvListContent>
						{SHORTCUTS.map((item) => {
							const binding = bindings[item.action];
							const capturingThis = capturing === item.action;
							return (
								<ItemRow
									key={item.action}
									icon={<ItemIcon>{ACTION_ICONS[item.action]}</ItemIcon>}
									title={item.label}
									description={item.description}
									actions={
										<>
											<kbd
												aria-label={`${item.label}的当前快捷键`}
												className="min-w-[76px] rounded-md border
													border-hairline bg-[var(--husk-bg)] px-2 py-1
													text-center text-[11px] text-neutral-600
													[font-family:inherit]"
											>
												{capturingThis
													? '按下组合…'
													: bindingLabel(binding) || '未设置'}
											</kbd>
											<Button
												type="button"
												size="sm"
												variant="ghost"
												aria-label={`重新绑定${item.label}`}
												onClick={() =>
													setCapturing(capturingThis ? null : item.action)
												}
											>
												{capturingThis ? '取消' : '重新绑定'}
											</Button>
											<Button
												type="button"
												size="sm"
												variant="ghost"
												aria-label={`清除${item.label}的快捷键`}
												disabled={!binding}
												onClick={() => {
													if (!clearBinding(item.action))
														toast.error('保存失败，已保留原快捷键');
												}}
											>
												清除
											</Button>
										</>
									}
								/>
							);
						})}
					</KvListContent>
				</KvList>
			</SettingsSection>
		</div>
	);
}
