<script lang="ts">
	import {
		Empty,
		EmptyContent,
		EmptyDescription,
		EmptyHeader,
		EmptyTitle
	} from '$lib/components/ui/empty';

	let {
		title,
		description
	}: {
		title: string;
		description: string;
	} = $props();
</script>

<Empty class="rounded-xl border border-dashed bg-card">
	<EmptyHeader>
		<EmptyTitle>{title}</EmptyTitle>
		<EmptyDescription>{description}</EmptyDescription>
	</EmptyHeader>
	<EmptyContent></EmptyContent>
</Empty>
