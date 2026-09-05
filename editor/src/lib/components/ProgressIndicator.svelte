<script lang="ts">
	import { Progress } from '@stump/ui/components/ui/progress';
	import { progressPercent } from '$lib/ingest/helpers';
	import { getEditorSession } from '$lib/editor/session.svelte';

	let {
		dropItemId,
		analysisJobId,
		compact = false
	}: {
		dropItemId?: string | null;
		analysisJobId?: string | null;
		compact?: boolean;
	} = $props();

	const session = getEditorSession();
	let patch = $derived(
		dropItemId
			? session.progress.items[dropItemId]
			: analysisJobId
				? session.progress.jobs[analysisJobId]
				: undefined
	);
	let percent = $derived(patch ? progressPercent(patch.completed, patch.total) : 0);

	function humanizePhase(phase: string): string {
		return phase.toLowerCase().replaceAll('_', ' ').replace(/(^|\s)\S/g, (letter) => letter.toUpperCase());
	}
</script>

{#if patch}
	<div class={compact ? 'flex min-w-28 items-center gap-2' : 'flex flex-col gap-1.5'}>
		<div class="flex items-center justify-between gap-2 text-xs text-muted-foreground">
			<span>{patch.message ?? humanizePhase(patch.phase)}</span>
			<span class="tabular-nums">{percent}%</span>
		</div>
		<Progress value={percent} aria-label={`${percent}% complete`} />
	</div>
{/if}


