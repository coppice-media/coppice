<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';

	// The app has no landing page of its own; Devices is the home screen.
	$effect(() => {
		void goto(resolve('/devices'), { replaceState: true });
	});
</script>
