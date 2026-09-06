<script lang="ts">
	import { Card, CardDescription, CardHeader, CardTitle } from '@stump/ui/components/ui/card';

	let { label, value, hint }: { label: string; value: string; hint?: string } = $props();
</script>

<Card>
	<CardHeader class="gap-0.5 pb-2">
		<CardDescription>{label}</CardDescription>
		<CardTitle class="text-3xl tabular-nums">{value}</CardTitle>
		{#if hint}
			<span class="text-xs text-muted-foreground">{hint}</span>
		{/if}
	</CardHeader>
</Card>
