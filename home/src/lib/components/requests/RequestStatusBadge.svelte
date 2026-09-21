<script lang="ts">
	import { Badge } from '@stump/ui/components/ui/badge';
	import { REQUEST_STATUS_VARIANTS, statusLabel } from '$lib/requests';

	let { status }: { status: string | null | undefined } = $props();
	const variant = $derived(REQUEST_STATUS_VARIANTS[status?.toUpperCase() ?? ''] ?? 'outline');
</script>

<Badge {variant}>{statusLabel(status)}</Badge>
