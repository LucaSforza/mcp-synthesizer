# Task: Configurare dinamicamente la context window in queue_controller

## Obiettivo

Attualmente la dimensione della context window è hardcodata in più punti tramite il valore:

```rust
.env("CLAUDE_CODE_AUTO_COMPACT_WINDOW", "100000")
```

e viene inoltre configurata separatamente nello script `sbatch`.

Voglio introdurre un'unica configurazione centralizzata tramite un nuovo flag CLI del `queue_controller`.

---

## Requisiti

### Nuovo flag CLI

Aggiungere al binary `queue_controller` un nuovo flag:

```text
--ctx-size <INTEGER>
```

Caratteristiche:

- opzionale
- tipo: integer
- default: `200000`

Esempi:

```bash
queue_controller ...
```

usa automaticamente:

```text
ctx_size = 200000
```

mentre:

```bash
queue_controller --ctx-size 300000 ...
```

usa:

```text
ctx_size = 300000
```

---

## Propagazione del valore

Il valore di `ctx_size` deve essere utilizzato in tutti i punti in cui oggi viene impostata la context window.

### Claude Code

Sostituire ogni hardcode di:

```rust
.env("CLAUDE_CODE_AUTO_COMPACT_WINDOW", "100000")
```

con il valore proveniente dal nuovo flag `--ctx-size`.

L'environment variable deve continuare a chiamarsi:

```text
CLAUDE_CODE_AUTO_COMPACT_WINDOW
```

ma il valore deve essere dinamico.

---

### Slurm / sbatch

Individuare dove viene configurata la context window nello script sbatch generato dal queue controller.

Eliminare qualsiasi valore hardcoded e utilizzare lo stesso valore proveniente da:

```text
--ctx-size
```

In questo modo Claude Code e il job Slurm ricevono sempre la stessa configurazione.

---

## Refactoring richiesto

- Evitare duplicazione della logica.
- Il valore deve essere letto una sola volta dalla CLI.
- Propagare il parametro attraverso le funzioni già esistenti invece di introdurre nuove costanti globali.
- Non modificare comportamenti non correlati.

---

## Verifica

Verificare che:

### Caso default

```bash
queue_controller ...
```

produca:

```text
ctx_size = 200000
```

sia per Claude Code sia per sbatch.

### Caso custom

```bash
queue_controller --ctx-size 300000 ...
```

produca:

```text
ctx_size = 300000
```

sia per Claude Code sia per sbatch.

---

## Criteri di accettazione

- Compila con `cargo build`.
- Nessun valore hardcoded residuo relativo alla context window.
- Un'unica sorgente di verità: `Args::ctx_size`.
- Claude Code e sbatch ricevono sempre lo stesso valore.
