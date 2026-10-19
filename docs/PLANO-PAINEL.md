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

Uma lista ordenada em três trechos. A ordem no arquivo é a ordem na tela;
omitir um módulo = desligá-lo.

```yaml
panel:
  edge: left          # left | right | top | bottom
  column: 40          # px de conteúdo por coluna
  gap: 10             # entre colunas; entre linhas é sempre 5
  padding: 5          # vertical = 3×40 + 2×10 + 2×5 = 150 px; horizontal = 50 px
  grid: 5             # toda medida é múltipla disto
  maintenance: false  # true desenha a grade de 5 px atrás do painel
  monitor: 0
  reserve: true
  start:              # topo (vertical) ou esquerda (horizontal)
    - menu
    - windows
  center: []
  end:                # base (vertical) ou direita (horizontal)
    - clock:
        cities: [LAX, JFK, LHR, CDG]
        rotate: 5s
    - calendar
    - weather:
        model: icon_seamless
    - forecast:
        days: 4
    - tray
    - volume
    - brightness
    - workspaces
    - network
    - keyboard
    - battery
    - session
```

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
| | *já pronto (19/10)* | *modo manutenção (grade de 5 px, `maintenance: true`); monitor por nome da saída (`monitor: HDMI-A-1`); desenvolvimento no monitor secundário com `examples/maintenance.yaml`* |
| 1 | Módulos sem serviço | `menu`, `clock`, `calendar`, `session` |
| 2 | Clima | `weather` e `forecast` (HTTP, cache, ícones) |
| 3 | Módulos por evento | `volume`, `brightness`, `battery`, `network`, `keyboard` |
| 4 | Compositor | `workspaces`, `windows` (sway IPC, atrás da interface) |
| 5 | Bandeja | `tray` (StatusNotifierWatcher próprio) |
| 6 | Troca | paridade com o eww → o sway passa a subir `orbit-dock` + `orbit-panel` |

## 7. Pendências e riscos conhecidos

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
