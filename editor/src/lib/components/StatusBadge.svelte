<script lang="ts">
	import { Badge } from '@stump/ui/components/ui/badge';
	import { humanize, statusTone } from '$lib/ingest/helpers';

	let { status }: { status: string } = $props();
	let tone = $derived(statusTone(status));
</script>

<Badge variant={tone}>{humanize(status)}</Badge>
