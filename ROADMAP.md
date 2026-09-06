# perfo — roadmap de features

Contexto: htop resolve a TUI (usar ele). O perfo é o que o htop NÃO tem:
widget do Omarchy + engine de dados + focos com processos.

## Engine de dados (`perfo <foco> --json`)
- [x] CPU: % por núcleo, load avg, memória, processos (pid/ppid/user/cpu%/mem/cmd)
- [x] CPU: detecção de threads (owner) e kernel (is_kernel)
- [x] CPU: last_cpu (núcleo onde o processo rodou por último)
- [ ] Rede: uso por interface + processos
- [ ] Disco: uso + processos + iowait
- [ ] Disco (futuro): io pressure, erros de driver
- [ ] Bateria: status detalhado
- [ ] Modo streaming pro widget (JSON contínuo, sem respawn a cada tick)

## Plugin Omarchy (vitor.perfo)
- [ ] Estrutura do plugin: manifest.json + validação (`omarchy plugin validate`)
- [ ] BarWidget: label CPU/RAM na barra, atualização periódica
- [ ] Panel: abre ao clicar no bar-widget
- [ ] Panel: barras por núcleo
- [ ] Panel: lista de processos (top por CPU)
- [ ] Panel: visão por núcleo (processos do núcleo selecionado)
- [ ] Ativação na barra + teste final

## Extras / decisões
- [ ] TUI do perfo (protótipo Rust): manter como reserva ou apagar
- [ ] lsof → dentro do foco Disco (quando o foco existir)
- [ ] Instalar `perfo` no PATH pra o widget conseguir chamar (`cargo install --path .`)