use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::Lang;
use crate::theme::Theme;

/// One row of help content.
///
/// `key` is the yellow key column (empty for non-key rows); `text` is the
/// description. Rows tagged `Colors` run their text through [`colorized`],
/// which understands `g:`/`y:`/`r:`/`a:` prefixes per `|`-separated segment.
pub struct HelpRow<'a> {
    pub kind: RowKind,
    pub key: &'a str,
    pub text: &'a str,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Hdr,
    Key,
    Txt,
    Colors,
}

/// Renders rows: headers in accent+bold, keys in yellow, threshold words
/// in their semantic color, everything else in the theme foreground.
pub fn help_lines<'a>(rows: &'a [HelpRow<'a>], theme: &Theme) -> Vec<Line<'static>> {
    rows.iter()
        .map(|r| match r.kind {
            RowKind::Hdr => Line::from(Span::styled(
                format!("  {}", r.text),
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            )),
            RowKind::Key => Line::from(vec![
                Span::styled(
                    format!("  {:<12}", r.key),
                    Style::default().fg(theme.yellow),
                ),
                Span::styled(r.text.to_string(), Style::default().fg(theme.fg)),
            ]),
            RowKind::Txt => Line::from(Span::styled(
                r.text.to_string(),
                Style::default().fg(theme.fg),
            )),
            RowKind::Colors => colorized(theme, r.text),
        })
        .collect()
}

/// "g:verde <5 | y:amarelo 5-10 | r:vermelho >10" -> colored spans; `a:` maps
/// to the accent (used for "blue" in the memory legend).
fn colorized(theme: &Theme, s: &str) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for part in s.split('|') {
        let p = part.trim();
        let (col, txt) = match p.strip_prefix("g:") {
            Some(x) => (theme.green, x),
            None => match p.strip_prefix("y:") {
                Some(x) => (theme.yellow, x),
                None => match p.strip_prefix("r:") {
                    Some(x) => (theme.red, x),
                    None => match p.strip_prefix("a:") {
                        Some(x) => (theme.accent, x),
                        None => (theme.fg, p),
                    },
                },
            },
        };
        spans.push(Span::styled(txt.to_string(), Style::default().fg(col)));
        if !spans.is_empty() {
            spans.push(Span::raw("   "));
        }
    }
    Line::from(spans)
}

pub fn page(page: usize, lang: Lang, theme: &Theme) -> (String, Vec<Line<'static>>) {
    let (title, rows): (String, &[HelpRow<'static>]) = match (lang, page) {
        (Lang::Pt, 0) => (
            "help 1/5 — teclas".into(),
            &[
                hdr("NAVEGACAO"),
                key("m", "abre o menu de paineis"),
                key("1-6", "abre o painel em tela cheia; repetir volta ao dashboard"),
                key("Tab", "na CPU alterna o foco entre CORES e PROCESSOS"),
                key("←↑↓→", "com CORES focado, muda o nucleo selecionado"),
                key("↑ ↓", "com PROCESSOS focado, seleciona um processo"),
                key("Enter", "filtra pelo nucleo selecionado; Esc remove o filtro"),
                txt(""),
                hdr("ACOES GERAIS"),
                key("p/M", "ordena processos por CPU ou MEMORIA"),
                key("i", "inverte a ordem da tabela"),
                key("c", "alterna comando curto/completo"),
                key("t", "alterna visualizacao em arvore"),
                key("H / K", "mostra threads / threads do kernel"),
                key("/", "busca por texto no comando do processo"),
                key("z", "pausa/retoma a coleta de dados"),
                key("s / k", "trace de syscalls / kill do processo selecionado"),
                key("C / L / q", "troca tema / idioma PT-EN / sai"),
                txt(""),
                hdr("HELP"),
                key("n / p", "proxima/anterior pagina; PgUp/PgDn tambem"),
                key("? / q", "fecha o help"),
            ],
        ),
        (Lang::En, 0) => (
            "help 1/5 — keys".into(),
            &[
                hdr("NAVIGATION"),
                key("m", "open the panel menu"),
                key("1-6", "open fullscreen; press the same number to return"),
                key("Tab", "in CPU, switch focus between CORES and PROCESSES"),
                key("←↑↓→", "with CORES focused, move the selected core"),
                key("↑ ↓", "with PROCESSES focused, select a process"),
                key("Enter", "filter by selected core; Esc clears the filter"),
                txt(""),
                hdr("GENERAL ACTIONS"),
                key("p/M", "sort processes by CPU or MEMORY"),
                key("i", "invert table order"),
                key("c", "toggle short/full command"),
                key("t", "toggle tree view"),
                key("H / K", "show threads / kernel threads"),
                key("/", "search process command text"),
                key("z", "pause/resume data collection"),
                key("s / k", "syscall trace / kill the selected process"),
                key("C / L / q", "change theme / language PT-EN / quit"),
                txt(""),
                hdr("HELP"),
                key("n / p", "next/previous page; PgUp/PgDn also work"),
                key("? / q", "close help"),
            ],
        ),
        (Lang::Pt, 1) => (
            "help 2/5 — bloco CPU".into(),
            &[
                hdr("CPU — VISAO GERAL"),
                key("overall", "uso total de todos os nucleos (barra + %)"),
                key("load 1/5/15", "processos prontos ou esperando I/O"),
                key("iowait", "tempo ocioso esperando disco; nao e uso de CPU"),
                txt(""),
                hdr("CORES E CORES"),
                cols("g:P Performance: mais rapido, L2 privado | y:E Efficient: compartilha L2 | r:L Low-power: economiza bateria"),
                txt("a letra aparece antes de cada nucleo; a cor indica o tipo"),
                txt(""),
                hdr("LEITURA DOS VALORES"),
                cols("GHz = frequencia atual; a cor compara com o limite do proprio nucleo"),
                cols("g:<33% do limite | y:33-66% | r:>66%"),
                cols("temperatura: g:<50°C | y:50-80°C | r:>80°C"),
                txt("100% de CPU significa um nucleo totalmente ocupado"),
                txt(""),
                hdr("TABELA DE PROCESSOS"),
                txt("PID | USER | CPU% | MEM | COMMAND; as setas selecionam"),
                txt("CPU% e por nucleo, e o destaque mostra o processo escolhido"),
            ],
        ),
        (Lang::En, 1) => (
            "help 2/5 — CPU block".into(),
            &[
                hdr("CPU — OVERVIEW"),
                key("overall", "usage across all cores (bar + %)"),
                key("load 1/5/15", "tasks ready to run or waiting for I/O"),
                key("iowait", "idle time waiting for disk; not CPU usage"),
                txt(""),
                hdr("CORE TYPES AND COLORS"),
                cols("g:P Performance: fastest, private L2 | y:E Efficient: shared L2 | r:L Low-power: saves battery"),
                txt("the letter before each core identifies its type"),
                txt(""),
                hdr("READING THE VALUES"),
                cols("GHz = current frequency; color compares it with that core's own limit"),
                cols("g:<33% of limit | y:33-66% | r:>66%"),
                cols("temperature: g:<50°C | y:50-80°C | r:>80°C"),
                txt("100% CPU means one whole core is busy"),
                txt(""),
                hdr("PROCESS TABLE"),
                txt("PID | USER | CPU% | MEM | COMMAND; arrows select a process"),
                txt("CPU% is per core, and the highlight marks the selected process"),
            ],
        ),
        (Lang::Pt, 2) => (
            "help 3/5 — memoria e discos".into(),
            &[
                hdr("4:MEM — PAINEL DE MEMORIA"),
                key("used", "RAM usada por aplicativos; nao inclui cache/buffers"),
                key("cache", "cache de paginas; pode ser liberado pelo kernel"),
                key("buffers", "buffers usados por operacoes de I/O"),
                key("available", "estimativa de RAM que pode ser usada agora"),
                cols("verde usado | amarelo cache | azul buffers | cinza livre"),
                txt("RAM usada alta sozinha nao e problema; observe available e PSI"),
                txt(""),
                hdr("SWAP E PSI"),
                key("swap", "memoria de troca usada/total e percentual"),
                key("PSI", "tempo em que processos ficaram esperando memoria"),
                cols("PSI: g:<5% normal | y:5-10% atencao | r:>10% pressao"),
                txt("os tres valores do PSI sao medias de 10s, 60s e 300s"),
                txt(""),
                hdr("TOP MEMORY PROCESSES"),
                txt("mostra os processos reais que mais ocupam RAM"),
                txt("threads e processos do kernel ficam fora dessa lista"),
                txt("use M no painel CPU para ordenar a tabela completa por memoria"),
            ],
        ),
        (Lang::En, 2) => (
            "help 3/5 — memory and disks".into(),
            &[
                hdr("4:MEM — MEMORY PANEL"),
                key("used", "RAM used by applications; excludes cache/buffers"),
                key("cache", "page cache; the kernel can reclaim it"),
                key("buffers", "buffers used by I/O operations"),
                key("available", "estimate of RAM that can be used now"),
                cols("green used | yellow cache | blue buffers | gray free"),
                txt("high used RAM alone is not a problem; watch available and PSI"),
                txt(""),
                hdr("SWAP AND PSI"),
                key("swap", "used/total swap and percentage"),
                key("PSI", "time processes spent waiting for memory"),
                cols("PSI: g:<5% normal | y:5-10% watch | r:>10% pressure"),
                txt("the three PSI values are 10s, 60s, and 300s averages"),
                txt(""),
                hdr("TOP MEMORY PROCESSES"),
                txt("shows the real processes using the most RAM"),
                txt("threads and kernel processes are excluded from this list"),
                txt("use M in the CPU pane to sort the full table by memory"),
            ],
        ),
        (Lang::Pt, 3) => (
            "help 4/5 — discos e rede".into(),
            &[
                hdr("5:DISKS — ESPACO"),
                txt("lista cada montagem com dispositivo, mountpoint e filesystem"),
                txt("USED/TOTAL = espaco ocupado e capacidade total"),
                cols("g:<70% normal | y:70-85% atencao | r:>85% cheio"),
                txt("o dashboard agrupa subvolumes btrfs do mesmo dispositivo"),
                txt(""),
                hdr("DEVICE DETAILS"),
                txt("mostra temperatura, leitura/escrita atuais e totais desde o boot"),
                txt("temperatura: g:<55°C | y:55-70°C | r:>=70°C"),
                txt("temperatura pode nao aparecer se o hardware nao expuser sensor"),
                txt(""),
                hdr("3:NET — REDE"),
                txt("por interface: RX/s, TX/s, pacotes/s, erros, drops e link"),
                txt("TCP retrans = retransmissoes; conexoes = TCP estabelecidas"),
                txt("PORTAS ABERTAS aparece somente em tela cheia"),
                txt(""),
                hdr("6:GPU"),
                txt("uso, VRAM, temperatura e potencia quando o driver fornece"),
                txt("GPU PROCESSES: GPU%, CPU%, RAM%, VRAM, usuario e comando"),
                txt("GPU integrada usa RAM compartilhada; VRAM por processo pode ser --"),
            ],
        ),
        (Lang::En, 3) => (
            "help 4/5 — disks and network".into(),
            &[
                hdr("5:DISKS — SPACE"),
                txt("lists each mount with device, mountpoint, and filesystem"),
                txt("USED/TOTAL = occupied space and total capacity"),
                cols("g:<70% normal | y:70-85% watch | r:>85% full"),
                txt("the dashboard groups btrfs subvolumes from one device"),
                txt(""),
                hdr("DEVICE DETAILS"),
                txt("shows temperature, current read/write, and boot totals"),
                txt("temperature: g:<55°C | y:55-70°C | r:>=70°C"),
                txt("temperature may be absent if hardware exposes no sensor"),
                txt(""),
                hdr("3:NET — NETWORK"),
                txt("per interface: RX/s, TX/s, packets/s, errors, drops, and link"),
                txt("TCP retrans = retransmissions; connections = established TCP"),
                txt("LISTENING PORTS appears only in fullscreen"),
                txt(""),
                hdr("6:GPU"),
                txt("usage, VRAM, temperature, and power when the driver provides them"),
                txt("GPU PROCESSES: GPU%, CPU%, RAM%, VRAM, user, and command"),
                txt("integrated GPUs use shared system RAM; per-process VRAM may be --"),
            ],
        ),
        (Lang::Pt, 4) => (
            "help 5/5 — IO: o que cada stat significa".into(),
            &[
                hdr("IOPS"),
                key("r/s w/s", "operacoes de leitura/escrita por segundo"),
                txt("IOPS alto com taxa baixa geralmente indica I/O aleatorio"),
                txt(""),
                hdr("LATENCIA"),
                key("r_awt/w_awt", "tempo medio da op em ms, fila incluida"),
                cols("g:<2ms saudavel | y:2-10ms atencao | r:>=10ms alta — NVMe bom fica <1ms"),
                txt(""),
                hdr("FILA E BUSY%"),
                key("fila", "requisicoes em voo (aqu-sz)"),
                cols("g:<2 normal | y:2-8 atencao | r:>=8 alta"),
                key("busy", "% do tempo com alguma operacao em voo"),
                txt("em NVMe, busy 100% nao significa saturacao; observe await e fila"),
                txt(""),
                hdr("PRESSAO E IOWAIT"),
                key("io pressure", "PSI: tempo em que I/O bloqueia processos"),
                key("iowait", "% ocioso esperando I/O (linha do CPU)"),
                cols("g:<5 normal | y:5-10 atencao | r:>10 estresse"),
                txt(""),
                hdr("I/O POR PROCESSO"),
                txt("read/write mostram bytes reais no storage, nao apenas cache"),
                txt("a lista mostra teus processos; root consegue ver todos"),
            ],
        ),
        (Lang::En, 4) => (
            "help 5/5 — IO: what each stat means".into(),
            &[
                hdr("IOPS"),
                key("r/s w/s", "read/write operations per second"),
                txt("high IOPS with low throughput usually means random I/O"),
                txt(""),
                hdr("LATENCY"),
                key("r_awt/w_awt", "average op time in ms, queue included"),
                cols("g:<2ms healthy | y:2-10ms watch | r:>=10ms high — healthy NVMe stays <1ms"),
                txt(""),
                hdr("QUEUE AND BUSY%"),
                key("queue", "in-flight requests (aqu-sz)"),
                cols("g:<2 normal | y:2-8 watch | r:>=8 high"),
                key("busy", "% of time with an operation in flight"),
                txt("on NVMe, busy 100% does not mean saturation; watch await and queue"),
                txt(""),
                hdr("PRESSURE AND IOWAIT"),
                key("io pressure", "PSI: time I/O stalls processes"),
                key("iowait", "% idle waiting on I/O (CPU line)"),
                cols("g:<5 normal | y:5-10 watch | r:>10 stressed"),
                txt(""),
                hdr("PER-PROCESS I/O"),
                txt("read/write show real storage bytes, not cache-only activity"),
                txt("the list shows your processes; root can see all processes"),
            ],
        ),
        (_, _) => unreachable!("help page out of range"),
    };
    (title, help_lines(rows, theme))
}

const fn hdr(text: &'static str) -> HelpRow<'static> {
    HelpRow {
        kind: RowKind::Hdr,
        key: "",
        text,
    }
}

const fn key(key: &'static str, text: &'static str) -> HelpRow<'static> {
    HelpRow {
        kind: RowKind::Key,
        key,
        text,
    }
}

const fn txt(text: &'static str) -> HelpRow<'static> {
    HelpRow {
        kind: RowKind::Txt,
        key: "",
        text,
    }
}

const fn cols(text: &'static str) -> HelpRow<'static> {
    HelpRow {
        kind: RowKind::Colors,
        key: "",
        text,
    }
}
