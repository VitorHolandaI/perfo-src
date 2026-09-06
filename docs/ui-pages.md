# Paginas da UI do Perfo

Este documento descreve as paginas do painel QML do plugin Omarchy. O painel
usa os dados emitidos por `perfo stream --json`. As paginas sao navegadas com
as setas esquerda/direita ou `h`/`l`; o contador no topo usa a forma `pagina/total`.

## Regra geral

- `--` significa que ainda nao existe uma amostra ou que o valor nao esta
  disponivel.
- Uma taxa e calculada entre duas amostras, aproximadamente uma vez por
  segundo no stream.
- Historicos sao buffers limitados a 120 amostras. O mais antigo sai quando o
  limite e atingido.
- Percentuais exibidos pela UI sao limitados ao intervalo de 0 a 100.
- Mount points, interfaces e nomes longos usam elision visual para nao
  empurrar as colunas vizinhas.
- O dashboard limita listas visuais para manter a janela legivel. O JSON
  pode conter mais processos, discos, interfaces ou fans do que a pagina mostra.

## 1. DASH

Objetivo: uma leitura rapida do estado geral da maquina.

### CPU

- `CPU N%` usa `overall_percent` de `sysinfo`, calculado a partir da variacao
  dos tempos de CPU entre refreshes.
- O grafico usa `cpu_history`, com uma barra para cada amostra recente.
- Cada barra tem altura proporcional ao percentual; picos curtos continuam no
  historico ate sairem do buffer.

### Memoria

- `MEM N%` calcula `used_mem_bytes * 100 / total_mem_bytes`.
- Esses dois campos sao os valores de memoria geral fornecidos pelo
  `sysinfo`; eles nao sao necessariamente iguais a `mem.used` da pagina MEM,
  que usa a formula explicita de `/proc/meminfo`.

### Load

- `LOAD N` mostra `load_avg[0]`, a carga media de um minuto obtida pelo
  `sysinfo`.
- Load nao e percentual de CPU. E uma medida de trabalho aguardando ou usando
  a CPU.

### Rede e disco

- `NET RX / TX` soma `rx_bps` e `tx_bps` das interfaces que sobreviveram ao
  primeiro delta de amostra.
- `IO READ / WRITE` soma `read_bps` e `write_bps` dos discos exibidos.
- `DISK / N%` mostra o primeiro filesystem retornado pelo coletor e seu
  percentual de espaco usado.

### Fans e processos

- `FANS N` mostra quantos fans foram encontrados em `hwmon`.
- `TOP PROCESSES` mostra ate quatro processos em duas colunas, ordenados por
  `cpu_percent` decrescente.
- O nome visual usa o campo curto `name` de `/proc`, e o percentual vem da
  variacao de CPU do processo entre refreshes.

## 2. CPU

Objetivo: mostrar tendencia de CPU e distribuicao por processador logico.

### CPU HISTORY

- Usa o mesmo `cpu_history` do dashboard.
- A escala e 0 a 100%; o maior valor recente aparece como a maior barra.

### PER-CORE

- Cada barra usa um item de `per_core`.
- `per_core[i]` e o uso percentual do CPU logico `i` no ultimo intervalo.
- O JSON tambem fornece `per_core_types`, `per_core_freq_mhz`,
  `per_core_max_freq_mhz` e `per_core_temp_c`.
- Esses metadados ja estao no contrato JSON, mas a pagina QML atual ainda
  mostra apenas as barras, sem rotulos individuais de tipo, MHz ou temperatura.

### Load, IOWAIT e temperatura

- `LOAD` repete a carga media de um minuto.
- `IOWAIT` usa os contadores agregados da linha `cpu` em `/proc/stat`: o
  coletor divide o tempo acumulado em `iowait` pelo total acumulado e multiplica
  por 100. No estado atual, isso e uma proporcao desde o boot, nao uma janela
  delta entre duas leituras; a documentacao registra essa diferenca para nao
  confundir o valor com uma taxa instantanea.
- `TEMP` e a maior temperatura encontrada entre componentes com label
  `Package`, `Core` ou `PECI`.

### Top processes

- `TOP PROCESSES` mostra ate oito processos em duas colunas, ordenados por
  `cpu_percent` decrescente.
- A pagina CPU mostra a lista completa do foco; o dashboard usa uma versao
  compacta com ate quatro processos para preservar o rodape.

## 3. IO

Objetivo: responder se o armazenamento esta trabalhando e se existe fila.

### READ e WRITE

- O painel soma `read_bps` e `write_bps` dos discos exibidos.
- O coletor converte o delta de bytes entre duas leituras de uso do disco em
  bytes por segundo.
- Um primeiro refresh nao tem amostra anterior; por isso a taxa inicial pode
  aparecer como `0 B/s`.

### PRESSURE

- Os tres numeros sao `io_pressure_some[0..2]`.
- A origem e `/proc/pressure/io`, na linha `some`:
  - `avg10`: media dos ultimos 10 segundos.
  - `avg60`: media dos ultimos 60 segundos.
  - `avg300`: media dos ultimos 300 segundos.
- O valor representa tempo em que alguma tarefa ficou parada por falta de I/O,
  nao percentual de ocupacao do disco.

### Grafico

- O grafico atual usa o historico de leitura do primeiro disco encontrado em
  `io_history`.
- A altura e normalizada pelo maior valor do proprio historico.
- O coletor tambem guarda historico de escrita, mas a pagina ainda nao mostra
  a serie de escrita separadamente.

### Linhas de disco

- A pagina mostra ate cinco filesystems reais.
- A primeira coluna e `mount`.
- A segunda e `R read_bps`.
- A terceira e `W write_bps`.
- As larguras sao fixas para que `/`, `/home` e mounts longos mantenham `R` e
  `W` no mesmo x.
- O JSON tambem possui IOPS, await, fila, busy, merges, tamanho medio,
  discard/TRIM, flush e temperatura; esses detalhes continuam disponiveis para
  uma futura pagina IO expandida.

## 4. NET

Objetivo: distinguir trafego agregado de trafego por interface.

### Grafico

- A linha superior representa o historico agregado de RX.
- A linha inferior representa o historico agregado de TX.
- Cada amostra soma as taxas das interfaces validas naquele intervalo.

### Totais

- `RX` e `TX` sao bytes por segundo derivados de deltas em `/proc/net/dev`.
- O primeiro refresh apenas prepara os contadores anteriores.
- `session_rx_bytes` e `session_tx_bytes` existem no JSON e acumulam bytes
  desde que o monitor iniciou, mas ainda nao aparecem nesta pagina.

### Interfaces

- A lista mostra ate cinco interfaces, ordenadas por RX atual.
- Para cada interface, `rx_bps` e `tx_bps` sao deltas de `rx_bytes` e `tx_bytes`.
- O coletor tambem calcula packets/s, erros/s, drops/s, link Mbps e estado do
  carrier; a UI atual mostra somente RX e TX.
- Interfaces virtuais como `lo`, `veth` e `virbr` podem ter taxa valida, mas
  normalmente nao possuem velocidade ou conceito de carrier.

## 5. MEM

Objetivo: explicar onde a memoria esta sendo usada, nao apenas mostrar um
percentual.

### Percentual e barra

- O numero grande usa `used_mem_bytes / total_mem_bytes * 100`.
- A barra usa o mesmo percentual e limita o resultado a 0..100.
- `total_mem_bytes` e `used_mem_bytes` vem do refresh de memoria do `sysinfo`.

### USED e AVAILABLE

- `USED` mostra `mem.used` formatado em B, K, M ou G.
- O coletor calcula `mem.used` a partir de `/proc/meminfo`:

  `used = MemTotal - MemFree - Buffers - cache`

- A cache e:

  `cache = Cached + SReclaimable - Shmem`

- `AVAILABLE` mostra `MemAvailable`, que e a estimativa do kernel de quanto
  pode ser alocado sem iniciar swap pesado.

### Swap

- `SWAP used / total` usa:

  `swap_used = SwapTotal - SwapFree`

- Se nao houver swap, os valores permanecem zero, que e diferente de sensor
  indisponivel porque `/proc/meminfo` fornece os campos.

### Pressure

- `PRESSURE` usa `mem.psi_some_10`, `psi_some_60` e `psi_some_300`.
- A fonte e `/proc/pressure/memory`, linha `some`.
- Pressao indica tarefas paradas esperando memoria, nao a quantidade de RAM
  ocupada.

## 6. DISKS

Objetivo: mostrar capacidade dos filesystems montados.

- A fonte e `sysinfo::Disks`, filtrada para filesystems reais como btrfs, ext4,
  vfat, xfs, f2fs, ntfs, zfs, exfat e ext2.
- A pagina mostra ate cinco discos.
- `total_bytes` vem do espaco total do filesystem.
- `available_bytes` vem do espaco disponivel.
- `used_bytes = total_bytes - available_bytes`.
- `percent = used_bytes / total_bytes * 100`.
- A barra usa `percent`.
- O percentual fica urgente quando e maior ou igual a 90%.
- A temperatura tenta resolver o dispositivo real, particao pai e slaves de
  device-mapper antes de procurar `temp1_input` no hwmon do controlador.

## 7. FANS

Objetivo: mostrar temperatura de CPU e RPM somente quando o kernel disponibiliza
essa informacao.

### Temperatura

- `CPU N C` usa o maior valor entre componentes `Package`, `Core` e `PECI`.
- A temperatura vem de componentes detectados pelo `sysinfo`, normalmente
  apoiados em hwmon.
- Sem componente compativel, a UI mostra `CPU TEMP --`.

### Fans

- O coletor enumera cada `hwmon*/fanN_input` em `/sys/class/hwmon`.
- `fanN_label` e usado quando existe; caso contrario o nome vira `Fan N`.
- O valor e RPM direto do arquivo `fanN_input`; `0` e um valor valido e fica
  visivel como `0 RPM`.
- A UI mostra ate seis fans, com label, RPM e nome do chip.
- Uma deduplicacao conhecida prefere `cros_ec` sobre `acpi_fan` e prefere
  `acpi_fan` sobre `asus` quando as duas interfaces existem. Isso evita contar
  duas vezes a mesma ventoinha quando uma leitura ASUS/WMI apresenta valores
  instaveis; outros chips, como `nct6775`, continuam visiveis.
- Se nenhum `fanN_input` puder ser lido, o cabecalho mostra `NO READABLE
  COOLERS` e o JSON lista uma colecao vazia.

### Estado desta maquina

Na maquina usada para validar o plugin, o hwmon expoe duas interfaces para o
mesmo caminho provavel da ventoinha:

- `acpi_fan/fan1`: RPM lido de `fan1_input`.
- `asus/fan1`: label `cpu_fan` e RPM lido de `fan1_input`, mas a leitura
  observada variou de `1500` a `24300 RPM` em poucos segundos, enquanto
  `acpi_fan` permaneceu perto de `3500 RPM`.

O binario instalado em `~/.local/bin/perfo` agora inclui `fans` no JSON e o
coletor mantem apenas a fonte `acpi_fan` neste caso. Se a UI voltar a mostrar
`FANS 0`, o primeiro diagnostico deve ser confirmar se o stream em execucao e a
build atual, antes de concluir que o hardware nao tem fan legivel.

## Dados ainda sem pagina propria

- `gpu.devices` agora aparece na pagina GPU do painel, com uso por dispositivo,
  memoria quando fornecida pelo backend e processos associados por PID.
- A tela `6:GPU` tambem exibe CPU%, RAM%, GPU%, VRAM por processo, usuario e
  comando; em GPU integrada a VRAM por processo pode ser `--` por usar RAM
  compartilhada.
- `net.proc_net` e `net.listening` ja existem no JSON, mas ainda nao aparecem
  na pagina NET.
- Os detalhes avancados de disco e os metadados por core tambem estao no JSON
  e aguardam uma apresentacao visual dedicada.

## Fontes tecnicas

- [Linux hwmon sysfs interface](https://docs.kernel.org/hwmon/sysfs-interface.html)
- [Linux `/proc/stat`](https://man7.org/linux/man-pages/man5/proc_stat.5.html)
- [Linux `/proc/meminfo`](https://man7.org/linux/man-pages/man5/proc_meminfo.5.html)
- [Linux `/proc/net/dev`](https://man7.org/linux/man-pages/man5/proc_net.5.html)
- [Linux PSI](https://docs.kernel.org/accounting/psi.html)
- [sysinfo crate](https://docs.rs/sysinfo/latest/sysinfo/)
