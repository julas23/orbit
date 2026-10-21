# Plano do Orbit-Panel

Status: **rascunho para validação** (2026-10-19).
Origem: o painel em eww (`~/.config/eww`, espelho em `julas23/linux/services/eww-painel`),
validado como "esboço final" em 2026-10-19. Ele é a especificação viva deste plano.

## 1. Regras que não mudam

1. **Conteúdo sempre de pé.** Texto, números e ícones NUNCA são rotacionados, em
   nenhuma posição do painel. Mudar o painel de lugar muda o *arranjo* do conteúdo,
   jamais a orientação dele. Nada de `transform`/rotação em CSS ou em código.
2. **Todo módulo é independente e opcional.** Cada um é ligado, desligado e
   ordenado pela configuração. Nenhum módulo depende de outro para existir.
3. **Nada de processos para coletar dados.** Nenhum módulo dispara comando externo
   periodicamente. Os dados chegam por evento (D-Bus, IPC do compositor, PipeWire,
   inotify) ou por timer interno. Meta: ~0% de CPU com a tela parada.
4. **Dois modos por módulo**, escolhidos pela borda do painel (seção 2).

## 2. Medidas: colunas no eixo fixo, passos de 5 px no eixo livre

O monitor é mais largo do que alto. Por isso a espessura do painel não é simétrica:

- **Horizontal** (topo ou base): tem que ser fino. Cada pixel de altura sai do espaço
  das janelas. Espessura = **1 linha**.
- **Vertical** (esquerda ou direita): pode ser largo sem prejudicar o uso.
  Espessura = **3 colunas**, cada uma equivalente à altura do painel horizontal.

Dois eixos, tratados de forma diferente:

| Eixo | Medido em | Exemplo |
|---|---|---|
| **Fixo** (espessura do painel) | **colunas**: no vertical, um módulo ocupa 1, 2 ou 3; no horizontal, sempre a linha toda | rede, teclado e bateria = 1 coluna cada, lado a lado |
| **Livre** (comprimento do painel) | **passos de 5 px**: o módulo tem o tamanho que o conteúdo pede, arredondado para cima ao múltiplo de 5 | relógio com 85 px de altura no vertical, não 100 nem 150 |

**Grade de 5 px:** toda medida do painel é múltipla de 5 — tamanhos, vãos, paddings,
posições. Os 4 px do eww (vão entre cartões e padding) passam a **5 px**.

**Decidido (2026-10-19): painel vertical de 150 px**, que fecha a conta
`3 colunas + 2 vãos entre colunas + 2 paddings` com tudo múltiplo de 5:

| Medida | Valor |
|---|---|
| Coluna (conteúdo) | **40 px** |
| Vão entre colunas (módulos lado a lado) | **10 px** |
| Vão entre linhas (um cartão e o de baixo) | **5 px** |
| Padding do painel | **5 px** |
| **Painel vertical** | 3×40 + 2×10 + 2×5 = **150 px** |
| **Painel horizontal** | 40 + 2×5 = **50 px** |

O vão de 10 px só aparece onde módulos de 1 ou 2 colunas dividem uma linha (rede,
teclado, bateria). Tudo configurável (`column:`, `gap:`, `padding:`).
Antes valia coluna de 45 px / painel de 155 px; foi trocado para manter os 150 px
do eww.

Modos de cada módulo, escolhidos pela borda:

| Borda | Modo | O módulo declara |
|---|---|---|
| `left`, `right` | **bloco** | quantas colunas ocupa (1–3); a altura sai do conteúdo, em passos de 5 px |
| `top`, `bottom` | **linha** | só o conteúdo; a largura sai dele, em passos de 5 px |

No **modo bloco**, módulos de menos de 3 colunas se juntam na mesma linha, lado a
lado — o cartão "rede/teclado/bateria" do eww vira 3 módulos de 1 coluna.
No **modo linha**, o que não cabe numa linha vai para o **tooltip** ou para um
**popup** aberto no clique (calendário do mês, slider de volume).

### 2.1 Grade de 4 colunas (decidido em 2026-10-21)

Substitui as 3 colunas de 40 px no painel vertical. A grade tem 150 px, e o painel 154 com as margens:

| Medida | Valor |
|---|---|
| Célula | **30 × 30 px** |
| Vão entre colunas e entre linhas | **10 px** |
| Margem lateral | 2 px de cada lado: 2 + 4 × 30 + 3 × 10 + 2 = **154 px** (`size: 154`) |
| Padding em cima e embaixo | 5 px |
| Linhas | quantas couberem: **27** em 1080 px (5 + 27 × 30 + 26 × 10 + 5 = 1080) |

A grade dá o tamanho dos módulos, mas o desenho é mais justo (2026-10-21): módulos
empilhados ficam **5 px** um do outro (onde nenhum módulo atravessa a fronteira entre
duas linhas), e os de 1 coluna têm **34 px** de largura, os quatro de uma fila a 5, 4
e 5 px. O fundo do painel é transparente; cada módulo leva o seu.

Um módulo ocupa um bloco de células inteiras: 30, 70, 110 ou 150 px de largura;
30, 70, 110, 150… px de altura. Colunas `a`–`d`, linhas a partir de 1: a primeira
linha é a1 b1 c1 d1, a última a27 … d27. O modo manutenção desenha as células com
os nomes, e o log diz onde cada módulo caiu (`panel: clock at a12:d13`).

Cada módulo vem em até três tamanhos (`Mini`, `Normal`, `Full`), escolhidos no
`panel.yaml` (`- clock: mini`; sem tamanho, vale o padrão):

| Módulo | Full | Normal | Mini |
|---|---|---|---|
| `clock` | **4×2**: data, fuso + hora + ampulheta, cidades | 4×1: fuso + hora + ampulheta | 3×1: só a hora |
| `calendar` | **4×4**: estação/ano, cabeçalho, mês | 4×2: estação/ano, cabeçalho, esta semana e a próxima | 3×1: "qua 21 out" |
| `weather` | **4×4**: tudo + previsão dos próximos dias | 4×2: figura, temperaturas, descrição | 3×1: figura + temperatura |
| `volume`, `brightness` | 4×1: ícone + slider horizontal | **2×1**: ícone + slider vertical | 1×1: ícone |
| `network`, `keyboard`, `battery`, `notifications`, `bluetooth`, `monitors` | 4×2: ícone + até 4 linhas | **4×1**: ícone + 2 linhas | 1×1: ícone |

Em negrito, o padrão (o tamanho de hoje). `windows`, `workspaces` (4×2), `tray`,
`session` e `menu` têm um tamanho só; `windows` e `tray` crescem com o conteúdo.
`forecast` continua existindo sozinho, mas o `weather` Full já inclui a previsão.

Os módulos se encaixam em ordem, da esquerda para a direita e de cima para baixo,
cada um no primeiro bloco livre depois do anterior; a parte `end` termina na última
linha. Se o conteúdo pedir mais linhas do que o tamanho, o módulo ganha as linhas e
o log avisa. **Posição livre por célula fica para depois**, junto com o modo
manutenção interativo (mover e trocar de tamanho arrastando).

Interação, para todos os módulos e botões: hover mostra o que o tamanho esconde;
clique esquerdo, ações rápidas; clique direito, opções completas; roda do mouse
ajusta o valor quando há um (volume, brilho). Já feito no Mini de volume e brilho
(esquerdo: slider num popover; direito: o mixer) e nos tooltips de todos os
tamanhos menores, e nos módulos `bluetooth` e `monitors` (2026-10-21):

- `bluetooth` (BlueZ pelo D-Bus do sistema, por evento): esquerdo abre o switch do
  adaptador e um switch por dispositivo pareado (conectar/desconectar); direito, o
  `blueman-manager` (`manager:` em `modules/bluetooth.yaml`).
- `monitors` (IPC do compositor, evento `output`): esquerdo põe cada monitor acima,
  abaixo ou de um lado do principal (o do notebook), centrado; direito abre o editor,
  o layout em escala, onde o monitor arrastado encaixa no lado mais próximo de outro,
  e "Aplicar" envia. Os dois mandam `output NOME pos X Y`, que o orbit-core aplica na
  hora e guarda em `~/.local/state/orbit/core/outputs.yaml` para a próxima sessão.
  Espelhar e desligar monitor ainda não existem no core.

Falta padronizar os cliques dos status antigos (rede, teclado, bateria, notificações).

## 3. Os módulos

"Bloco" = painel vertical (colunas ocupadas, de 3); "linha" = painel horizontal.
Alturas no bloco e larguras na linha saem do conteúdo, em passos de 5 px.

| Módulo | Bloco (vertical) | Linha (horizontal) | Fonte dos dados |
|---|---|---|---|
| `menu` | 3 col — logo | logo | — (clique roda o lançador) |
| `windows` (janelas) | 3 col — uma entrada por janela: ícone + nome do app | ícones em fila (nome no tooltip) | IPC do compositor (eventos) |
| `clock` (relógio) | 3 col — data / fuso + hora + ampulheta / cidade do mundo | fuso + hora + ampulheta; data e cidades no tooltip | timer interno alinhado ao segundo |
| `calendar` | 3 col — grade 7×6 do mês | dia do mês; grade num popup | timer (vira o dia à meia-noite) |
| `weather` (clima) | 3 col — ícone, temperatura, mín/máx, descrição, sol/lua, umidade/vento | ícone + temperatura; resto no tooltip | HTTP a cada 15 min (Open-Meteo ICON), com cache |
| `forecast` (previsão) | 3 col — 4 dias | 4 dias (ícone + máxima) | mesma resposta do `weather` |
| `tray` (bandeja) | 3 col — ícones em grade, quebra de linha | ícones em fila | StatusNotifier via D-Bus (implementação própria) |
| `volume` | 3 col — slider + ícone | ícone; roda do mouse ajusta, clique abre slider | PipeWire/Pulse (eventos) |
| `brightness` (brilho) | 3 col — slider + ícone | ícone; roda do mouse ajusta | sysfs backlight + inotify; DDC/CI no desktop |
| `workspaces` (áreas) | 3 col — grade 2×2 | 4 áreas em fila | IPC do compositor |
| `network` (rede) | 1 col — ícone + estado | ícone | NetworkManager via D-Bus |
| `keyboard` (teclado) | 1 col — layout | layout | IPC do compositor (eventos de input) |
| `battery` (bateria) | 1 col — ícone + % (some se não houver bateria) | ícone + % (idem) | UPower via D-Bus |
| `session` (sessão) | 3 col — desligar / reiniciar / sair | ícone; ações num popup | — (clique) |

Os detalhes de cada módulo no eww (medidas, cores, comportamento da ampulheta,
regra do clima, aeroportos, grade do calendário) estão no `status.md` e no
`done.md` de `~/.config/eww/.claude/` e valem como especificação.

## 4. Configuração

Implementado em 2026-10-19. Tudo em `~/.config/orbit/`, um arquivo por parte:

```text
~/.config/orbit/
├── panel.yaml        posição, medidas, monitor, manutenção e a LISTA de módulos
├── dock.yaml         o dock
├── style.css         tema do usuário
└── modules/
    └── <módulo>.yaml opções de cada módulo (clock.yaml, menu.yaml, ...)
```

- Ligar um módulo = listá-lo no `panel.yaml`, em `start`, `center` ou `end`; a ordem
  da lista é a ordem na tela. Desligar = tirar da lista.
- O arquivo do módulo é opcional; sem ele, valem os padrões.
- Módulo desconhecido ou com erro no arquivo fica de fora com mensagem no log; o
  resto do painel sobe.

```yaml
# panel.yaml
edge: left          # left | right | top | bottom
size: 150
monitor: HDMI-A-1   # índice ou nome da saída
maintenance: false
start: [menu, windows]
center: []
end: [clock, calendar, weather, forecast, tray, volume, brightness,
      workspaces, network, keyboard, battery, session]
```

```yaml
# modules/clock.yaml — até 4 fusos, um por vez
world:
  - { code: LAX, name: Los Angeles, tz: America/Los_Angeles }
  - { code: JFK, name: Nova York,   tz: America/New_York }
  - { code: LHR, name: Londres,     tz: Europe/London }
  - { code: CDG, name: Paris,       tz: Europe/Paris }
rotate: 5
```

Medidas da grade (`column`, `gap`, `padding`, `grid`) entram no `panel.yaml` quando
o layout em colunas for implementado.

## 5. Arquitetura

- **`Module` (trait):** identifica o módulo, informa quantas colunas ocupa no modo
  bloco (1–3) e constrói o widget GTK para o modo pedido. Recebe as opções da config.
- **Layout:** no modo bloco, módulos com menos de 3 colunas fluem lado a lado até
  completar a linha; a altura de cada linha é a do módulo mais alto, arredondada ao
  passo de 5 px. No modo linha, tudo vai numa única fila, larguras arredondadas ao passo.
- **Serviços compartilhados:** uma única conexão por fonte (IPC do compositor, D-Bus
  de sessão e de sistema, PipeWire, cliente HTTP do clima), aberta só se algum módulo
  ligado precisar dela. Os módulos assinam os serviços; nenhum abre conexão própria.
- **Compositor atrás de uma interface:** áreas, janelas e teclado falam com uma
  camada de compositor. Hoje: sway (IPC). Depois: `orbit-core`.
- **Tema:** os tokens do `eww.scss` (cores, cartão, vão de 4 px, padding de 4 px)
  viram o `style.css` embutido.

## 6. Etapas

Cada etapa termina com medição de CPU e memória e com print comparando ao eww.

| # | Etapa | Entrega |
|---|---|---|
| 0 | Fundação | trait `Module`, layout em colunas + passo de 5 px, config em lista, escolha do modo pela borda, tema base. Testado nas 4 bordas. |
| | *já pronto (19/10)* | *modo manutenção (grade de 5 px, `maintenance: true`); monitor por nome da saída (`monitor: HDMI-A-1`); config em diretório (`panel.yaml`, `dock.yaml`, `modules/*.yaml`); desenvolvimento no monitor secundário com `--config examples/maintenance`* |
| | *já pronto (19/10)* | *módulo `clock` completo (modos bloco e linha, até 4 fusos, ampulheta alinhada aos algarismos)* |
| | *já pronto (19/10)* | *módulo `calendar` (grade 7×6, cartão de 150 px; modo linha com o mês num popup — clique ainda não testado)* |
| | *já pronto (19/10)* | *módulo `workspaces` (áreas globais 2×2, cartão de 75 px) e cliente nativo do IPC do sway (`sway.rs`), orientado a eventos — clique e troca por teclado ainda não testados* |
| | *já pronto (19/10)* | *módulo `session` (3 botões de 40×30 px) com confirmação nativa sobre a tela desfocada (`confirm.rs`) e passo "antes de sair" configurável — clique ainda não testado* |
| | *já pronto (19/10)* | *módulos `volume` (PulseAudio nativo, por evento) e `brightness` (backlight via poll do kernel + logind; DDC/CI via ddcutil, não testado). Meia largura, lado a lado: cartões de 65×45 px. Primeira versão do layout "módulos lado a lado" (meia largura)* |
| | *já pronto (19/10)* | *módulo `windows`: lista de janelas igual à do eww, em ordem geométrica (área → monitor em posição física → janela, de cima para baixo e da esquerda para a direita; minimizadas por último). Arrastar para reordenar saiu. Clique e menu ainda não testados* |
| | *já pronto (19/10)* | *módulo `tray`: StatusNotifierItem via D-Bus do GIO (watcher próprio, ou host do watcher existente — hoje o do eww), menus dbusmenu convertidos para menu GTK. Cliques ainda não testados. Na troca (etapa 6): subir o painel ANTES do blueman-applet* |
| | *já pronto (19/10)* | *módulos `weather` (115 px) e `forecast` (60 px), mesmos dados e visual do eww: wttr.in (local + UV) + Open-Meteo ICON, busca única compartilhada a cada 15 min via `ureq`, cache em ~/.cache/orbit/weather.json; ícones do brweather em ~/.local/share/orbit/weather (fora do repo). Faltam ainda `network`, `keyboard` e `battery` (o cartão de estado do eww)* |
| | *já pronto (19/10)* | *`orbit-dock` igual ao do eww (lista sincronizada, centralizado, grupos); ícones de arquivo carregados já reduzidos — dock de 146 para 53 MB. Ordem de partida: painel primeiro, dock depois. Falta a etapa 6 (troca do eww na partida do sway)* |
| | *já pronto (19/10)* | *módulos `network`, `keyboard`, `battery` (status): ícone 24 px + duas linhas, cartões de 40 px. Rede: nome / IP+prefixo, ícone é botão (esquerdo abre o nm-connection-editor, direito o menu do nm-applet). Teclado: idioma / complemento, clique troca o layout. Bateria: % / uptime; sem bateria mostra tomada + uptime. Agora sim todos os módulos do eww existem* |
| | *preparado (19/10)* | *etapa 6: `scripts/orbit-session` (painel → dock, fallback se o painel não subir em 3 s) e `scripts/install.sh` (~/.local/bin). A troca no sway desta máquina é o `~/.config/sway/trocar-para-orbit.sh` (desfaz com `voltar-para-eww.sh`) — testado numa cópia do HOME; quem dispara é o Juliano* |
| 1 | Módulos sem serviço | `menu`, `clock`, `calendar`, `session` |
| 2 | Clima | `weather` e `forecast` (HTTP, cache, ícones) |
| 3 | Módulos por evento | `volume`, `brightness`, `battery`, `network`, `keyboard` |
| 4 | Compositor | `workspaces`, `windows` (sway IPC, atrás da interface) |
| 5 | Bandeja | `tray` (StatusNotifierWatcher próprio) |
| 6 | Troca | paridade com o eww → o sway passa a subir `orbit-dock` + `orbit-panel` |

## 7. Pendências e riscos conhecidos

- ~~**Cores do sistema**~~ — **feito em 2026-10-19:** paletas clara e escura
  (`data/palette-*.css`) escolhidas pelo `color-scheme` do portal, com a `accent-color`
  do sistema, troca ao vivo. Junto: **idiomas** com Fluent (`data/i18n/*.ftl`, inglês
  como fonte, pt-BR traduzido).

- **Memória:** na primeira medição o `orbit-dock` usou 146 MB (eww inteiro: 110 MB).
  Investigar antes da etapa 1 para não construir em cima do problema.
- **Ícones do clima:** os PNGs do brweather vêm de temas de terceiros (Faenza,
  Weezle, Cairo-dock, CPTEC/INPE) com licenças diferentes ou desconhecidas. Não
  podem entrar num repositório público GPL sem verificação. Alternativas: carregar
  de um diretório configurável (fora do repo) ou adotar um conjunto de licença livre.
- **Popups no layer-shell:** o modo linha depende de popups (calendário, volume,
  sessão). Validar que funcionam no sway com `gtk4-layer-shell` logo na etapa 0.
- **Backend X11:** hp e think são só Wayland hoje. Decidir se o X11 continua
  sendo mantido — cada módulo novo teria que ser testado nele.
- **Fontes não seguem a grade.** Corpo de letra e altura de linha vêm da fonte; o que
  se arredonda ao passo de 5 px é a caixa do módulo, não o texto dentro dela.
- **Brilho por DDC/CI** (desktop) é lento por natureza; precisa rodar fora da
  thread da interface.
