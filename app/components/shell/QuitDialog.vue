<script setup lang="ts">
// Asks before quitting while sessions are live. Keep working is the default (it has the focus), Esc does the same.
import { buttonVariants } from '@/components/ui/button'

const quit = useQuitStore()
const open = computed({
  get: () => quit.liveSessions !== null,
  set: (v: boolean) => { if (!v && quit.liveSessions !== null) quit.keepWorking() },
})
</script>

<template>
  <AlertDialog v-model:open="open">
    <AlertDialogContent>
      <AlertDialogHeader>
        <AlertDialogTitle>Quit Dalan?</AlertDialogTitle>
        <AlertDialogDescription>
          {{ quit.liveSessions === 1 ? '1 session is' : `${quit.liveSessions} sessions are` }} still open. Quitting ends {{ quit.liveSessions === 1 ? 'it' : 'them' }}, and anything running in {{ quit.liveSessions === 1 ? 'it' : 'them' }} stops.
        </AlertDialogDescription>
      </AlertDialogHeader>
      <AlertDialogFooter>
        <AlertDialogCancel data-autofocus>Keep working</AlertDialogCancel>
        <AlertDialogAction :class="buttonVariants({ variant: 'destructive' })" @click="quit.quit()">Quit anyway</AlertDialogAction>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>
</template>
