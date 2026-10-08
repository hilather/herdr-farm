/* Real native child whose argv storage temporarily exposes a process-title race. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
int main(int argc, char **argv) {
    if (argc == 2 && !strcmp(argv[1], "--version")) {
        puts("codex-cli 0.154.0"); return 0;
    }
    /* After the arguments-change exec, remain stable for identity observation. */
    if (argc > 1) for (;;) pause();
    FILE *mode = fopen("../mode", "r");
    int mutation = mode ? fgetc(mode) : '0';
    if (mode) fclose(mode);
    if (mutation == 'w') {
        mode = fopen("../mode", "w");
        if (!mode) return 2;
        fputs("0", mode);
        fclose(mode);
        /* Match the released sandbox wrapper's public process identity,
           then restore the exact agent argv by exec in the same child. */
        execl("/bin/sh", "/bin/sh", "-c",
            "touch setup-pending; sleep 2; exec \"$1\"",
            "herdr-farm-worker-sandbox", argv[0], (char *)NULL);
        return 3;
    }
    /* Wait until the public readiness request proves initial identity passed. */
    while (access("readiness-started", F_OK) != 0) usleep(10000);
    if (mutation == 'a') {
        execl(argv[0], argv[0], "unexpected-argument", (char *)NULL);
        return 3;
    }
    if (mutation == 'x') {
        execl("/usr/bin/sleep", "sleep", "60", (char *)NULL);
        return 3;
    }
    if (mutation != '0') {
        /* Linux's proctitle path reads argv[0] when arg_end is non-NUL.
           Cover both an empty title and one without a NUL in the read page. */
        size_t length = strlen(argv[0]) + 1;
        char *start = argv[0];
        unsigned long low, high;
        size_t span = length;
        char line[512];
        FILE *maps = fopen("/proc/self/maps", "r");
        if (!maps) return 2;
        while (fgets(line, sizeof line, maps)) {
            if (sscanf(line, "%lx-%lx", &low, &high) == 2 &&
                (unsigned long)start >= low && (unsigned long)start < high) {
                span = high - (unsigned long)start;
                break;
            }
        }
        fclose(maps);
        char *saved = malloc(span);
        memcpy(saved, start, span);
        memset(start, 'x', span);
        if (mutation == 'e') start[0] = 0;
        start[length - 1] = 'x';
        sleep(2);
        memcpy(start, saved, span);
        free(saved);
    }
    for (;;) pause();
}
