# Zero code comments, enforced

Source files contain no comments at all; names, types, the glossary, the spec and ADRs carry explanation. A lint check fails the build on any comment token. This is stricter than pstack's policy by Johan's choice, to force code that explains itself.
