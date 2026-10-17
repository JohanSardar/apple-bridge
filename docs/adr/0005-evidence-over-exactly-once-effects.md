# Evidence over exactly-once effects

Apple effects cannot be made exactly-once: the window between an effect and recording its result is unavoidable. Commands therefore use prepared evidence, single-use fenced tickets, a durable dispatch-start guard in the Helper, and admission eras; an ambiguous effect becomes indeterminate and is never redispatched automatically. Exactly-once is promised only for replica application of journal changes.
