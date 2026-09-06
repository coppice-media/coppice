<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';

	// `/app` and `/app/` both land here; the Dashboard is the console's home
	// screen, so neither is a page of its own.
	$effect(() => {
		void goto(resolve('/dashboard'), { replaceState: true });
	});
</script>
