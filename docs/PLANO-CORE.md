# Plano do Orbit-Core

Status: **Fase 1 em andamento** (2026-10-21) — plano validado; primeiro passo feito (abaixo, "Andamento").
Origem: a decisão de 2026-10-20 de que o sway sai e o **orbit-core** assume a
tarefa: um WM-DE próprio, sobre Smithay, que junta o melhor de i3/sway, bspwm,
xmonad, dwm, spectrwm, awesome, Hyprland e niri, e o que um DE (GNOME, KDE)
entrega além da janela. A configuração atual do sway (`~/.config/sway/config`) e
os onze componentes do Orbit que já rodam sobre ele são a especificação viva do
mínimo que o core precisa fazer no primeiro dia.

## 1. Regras que não mudam

1. **Nada quebra na troca.** Painel, dock, blink, menu, atalhos e scripts que hoje
   falam com o sway continuam funcionando sem mudança de código: o core fala o IPC
   do sway/i3 (seção 5) como **camada de compatibilidade**. O caminho principal
   entre o core e os componentes são **protocolos Wayland padrão** (o método do
   COSMIC: o compositor só compõe, o resto são clientes); recurso novo entra por
   protocolo padrão ou, na falta dele, por extensão do IPC — nunca por
   incompatibilidade.
2. **O sway fica como sessão de resgate** no greetd até o core ser o uso diário
   por **duas semanas seguidas** sem voltar ao sway. Só então ele sai do padrão
   (e continua instalado).
3. **Zero custo parado.** Tela sem mudança = nenhum quadro desenhado, nenhum
   timer acordando. Animação, blur e efeitos só existem enquanto algo se mexe, e
   desligam sozinhos na bateria se configurado.
4. **Tudo é configuração, nada é obrigatório.** Layout, animação, blur, cantos,
   gestos, painel: cada um liga e desliga. Sem nenhum arquivo de configuração o
   core sobe com padrões sensatos (o "funciona sem configurar" do spectrwm).
5. **Configuração recarrega a quente**, sem perder janelas, e um erro nela nunca
   derruba a sessão: o core avisa (notificação + log) e segue com a última válida.
6. **Um crash de cliente nunca derruba o core**; um crash do core deve ser raro e,
   quando acontecer, o autologin do tty1 sobe o core de novo **travado**, não um TTY morto.
7. **Testável sem tela.** Todo comportamento de janela tem teste automatizado
   rodando o core em backend headless (o mesmo jeito que testamos os apps hoje
   com o sway headless).
8. **Licença GPL-3.0-or-later**, Rust, MSRV do workspace (1.92).

## 2. Identidade

- Binário: `orbit-core`. Sessão no greetd: `orbit` (arquivo
  `/usr/share/wayland-sessions/orbit.desktop`).
- Configuração: `~/.config/orbit/core.yaml` (+ `core.d/*.yaml` incluídos em
  ordem), no mesmo estilo YAML dos módulos do painel.
- Socket IPC: `$XDG_RUNTIME_DIR/orbit-ipc.<uid>.<pid>.sock`, exportado em
  **`SWAYSOCK`** e **`I3SOCK`** (é isso que faz os clientes atuais funcionarem) e
  em `ORBIT_SOCK`.
- Comando de controle: `orbitctl` (como `swaymsg`/`bspc`), com
  `orbitctl -t get_tree` aceitando a mesma sintaxe do `swaymsg`.
- Estado em `~/.local/state/orbit/core/` (sessão de janelas, layouts por
  workspace).
- Login e bloqueio: `orbit-greet` (seção 7), serviço PAM `/etc/pam.d/orbit-greet`.

## 3. Recursos por origem

O que cada WM/DE ensina e como entra no core. Coluna "Fase" remete à seção 8.

| Origem | Recurso | Como fica no Orbit | Fase |
|---|---|---|---|
| **i3 / sway** | Árvore de contêineres (split h/v, tabbed, stacked) | Layout `tree`, o padrão; mesmos comandos (`split`, `layout tabbed`, `focus parent`) | 1 |
| | Scratchpad | `move scratchpad` / `scratchpad show` (o painel já usa) | 1 |
| | IPC por socket com eventos | Compatível com o sway (seção 5) | 1 |
| | Critérios `[app_id=… class=… title=…]` e `for_window` | Mesma sintaxe nos comandos; regras em YAML (`rules:`) | 1 |
| | Modos de atalho (`mode "resize"`) | `modes:` na configuração | 1 |
| | Gaps, bordas `pixel N`, `smart_gaps` | `gaps:`, `border:` por workspace e por regra | 1 |
| **bspwm** | Particionamento binário (BSP) com inserção preselecionada | Layout `bsp`; `presel` por direção/razão | 3 |
| | Todo controle por comando externo (`bspc`) | `orbitctl` cobre 100% das ações; atalhos são só comandos | 1 |
| | Nós marcados, "receptáculos" | Marcas (`mark`) do sway + receptáculo vazio no BSP | 3 |
| **xmonad** | Layouts combináveis e trocáveis por workspace | Cada workspace tem seu layout; `layout next` cicla uma lista configurável | 3 |
| | Modificadores de layout (espelhar, sem bordas se 1 janela, magnificar) | `layout_options:` (`mirror`, `smart_borders`, `magnify`) | 3 |
| **dwm** | Mestre-pilha com `nmaster` e `mfact` | Layout `master` (`master_count`, `master_ratio`, lado do mestre) | 3 |
| | Tags: janela em várias tags, ver várias tags juntas | Fora por ora (decidido em 2026-10-22): talvez numa major version depois da versão final, como camada sobre workspaces (`tag`, `view_tags`) | — |
| | Monocle | Layout `monocle` (uma janela por vez, cicla com foco) | 3 |
| **spectrwm** | Funciona sem configurar; barra e regras simples | Padrões embutidos; `core.yaml` vazio = sessão usável | 1 |
| | Workspaces e layout por monitor | Workspaces presos a monitor (`output:` por workspace, como a sua `workspace 11..24`) | 1 |
| **awesome** | Regras e layouts programáveis | Regras em YAML com ações encadeadas; layouts por plugin (seção 6, `orbit-layout`) — sem embutir Lua | 3 |
| | Widgets ligados ao WM | Já resolvido pelo painel; o core só expõe eventos | — |
| **Hyprland** | Animações com curvas (abrir, fechar, mover, trocar workspace) | `animations:` com curvas bezier/mola, por evento | 4 |
| | Blur, sombras, cantos arredondados, opacidade por regra | `decoration:` + regras; blur só em superfícies que pedirem/regra | 4 |
| | Workspaces especiais (gaveta que desce sobre o atual) | Scratchpad nomeado: `scratchpad show name=term` | 3 |
| | Gestos de touchpad (3 dedos troca workspace) | `gestures:`, com acompanhamento do dedo, não só disparo | 4 |
| | `hyprctl` com JSON | Coberto pelo IPC + `orbitctl -r` | 1 |
| **niri** | Faixa de rolagem infinita de colunas | Layout `scroll`: janelas em colunas que rolam na horizontal, foco centraliza | 3 |
| | Visão geral (overview) | Exposição de todos os workspaces com miniatura (a mesma captura do blink) | 4 |
| **GNOME / KDE** | Portal (captura de tela, seletor de arquivos, compartilhar tela) | `xdg-desktop-portal-wlr` no começo; backend próprio depois | 2 / 5 |
| | Gerência de sessão, autostart XDG, tela de bloqueio | Autostart XDG; login e bloqueio são a mesma camada do core, o `orbit-greet` (seção 7); `ext-session-lock-v1` para lockers de terceiros | 2 |
| | Configuração de monitores pela interface | Monitores via `wlr-output-management` (funciona com `wdisplays`/`kanshi`) e depois app Orbit | 2 / 5 |
| | Notificações, teclado/idioma, áudio, rede | Já no painel; o core só fornece o necessário (layout de teclado por IPC) | — |
| | Arrastar janela entre monitores, "aero snap" | Flutuantes com snap nas bordas e em metades/quartos | 3 |
| **Orbit (próprio)** | Painel, dock, menu, blink, apps | Já existem; viram cidadãos de primeira classe via layer-shell + IPC | 1 |
| | Acesso remoto nativo | `orbit-remote-server` + `orbit-local-station` (seção 6) | 5 |
| | Esfera/espectro "Jarvis" no wallpaper | Camada `background` própria; sobe para `overlay` quando o orbit-aigent ativa | 5 |
| | Voz (orbit-aigent) | Ativação por atalho do core; ações via IPC | 5 |

**Fora do escopo** (decidido agora para não voltar): Lua/scripting embutido
(awesome) — a extensão é por IPC e plugins Rust; X11 como sessão — X só via
XWayland; efeitos 3D tipo cubo (Compiz).

## 4. Protocolos Wayland

Necessários na **Fase 1** (sem eles o que já existe para de funcionar):

| Protocolo | Por quê |
|---|---|
| `xdg-shell`, `xdg-decoration` (preferindo decoração do servidor) | Janelas comuns; borda desenhada pelo core |
| `wlr-layer-shell` | Painel, dock, notificações, wallpaper (Jarvis), menus do painel |
| `wl-data-device`, `primary-selection`, `wlr-data-control` | Copiar/colar; orbit-clip depende do data-control |
| `wlr-screencopy`, `ext-image-copy-capture` | blink (usa screencopy direto), miniaturas da overview |
| `xdg-output`, `wlr-output-management`, `fractional-scale`, `viewporter` | Monitores; o 4K em HDMI-A-4 com escala |
| `linux-dmabuf`, `wp-presentation-time`, `single-pixel-buffer` | Desempenho do vídeo (orbit-play com VA-API) |
| `wlr-foreign-toplevel-management`, `ext-foreign-toplevel-list` | Lista de janelas no painel/dock fora do IPC |
| `idle-notify`, `idle-inhibit` | Apagar tela; vídeo inibe |
| `text-input-v3`, `input-method-v2`, `virtual-keyboard`, `wlr-virtual-pointer` | Dead keys do us alt-intl em todo app; testes automatizados (o `vptr`) |
| `pointer-constraints`, `relative-pointer`, `pointer-gestures` | Jogos, gestos |
| `xdg-activation` | Um app trazer outro à frente (abrir arquivo no orbit-files) |
| `ext-workspace-v1` | Áreas de trabalho para o painel sem IPC (protocolo que nasceu no COSMIC e foi padronizado) |
| XWayland (via `xwayland-satellite` ou o do Smithay) | Apps X restantes (Steam no PVE, utilitários antigos) |

Na **Fase 2**: `ext-session-lock`, `security-context` (sandbox), `tearing-control`
(jogos), `content-type`, `cursor-shape`.

## 5. IPC compatível com o sway

**Papel: camada de compatibilidade, não o centro.** Ela existe para que nada
quebre no dia da troca e para scripts, `orbitctl` e clientes de terceiros
(waybar, autotiling). Os componentes do Orbit migram aos poucos para os
protocolos padrão — `ext-workspace-v1` (áreas), `ext-foreign-toplevel-list` +
`wlr-foreign-toplevel-management` (janelas), `ext-image-copy-capture` (blink) —
e com isso passam a funcionar também em niri, COSMIC e qualquer compositor que
os implemente. O IPC continua mantido, mas não ganha recurso que um protocolo
padrão já cubra.

O core implementa o **mesmo protocolo binário** do i3/sway (`i3-ipc` + JSON). O
critério de compatibilidade é objetivo: **tudo o que o Orbit usa hoje**,
levantado no código em 2026-10-20:

| Mensagem/evento | Quem usa |
|---|---|
| `RUN_COMMAND` | painel (áreas, janelas, sessão, volume, status), dock |
| `GET_TREE` | blink (geometria de janela), painel (lista de janelas) |
| `GET_WORKSPACES`, `GET_OUTPUTS` | painel (áreas por monitor) |
| `GET_INPUTS` | painel (layout de teclado) |
| `SUBSCRIBE` `window`, `workspace`, `output`, `input` | painel |

Comandos que precisam existir no dia 1 (usados pelo Orbit ou pela sua config):
`workspace number N`, `focus output X`, `[con_id=N] focus|kill|move scratchpad`,
`[app_id="(?i)…"] focus`, `input type:keyboard xkb_switch_layout next`, `exit`,
`floating enable`, `move position center`, `move container to workspace number N`,
`fullscreen`, `split h|v`, `layout tabbed|stacking|toggle split`, `resize`,
`reload`, `exec`, `gaps`, `border pixel N`.

Formato do `GET_TREE` igual ao do sway nos campos que os clientes leem (`id`,
`type`, `name`, `app_id`, `window_properties`, `rect`, `focused`, `nodes`,
`floating_nodes`, `pid`, `visible`), para que também `swaymsg`, `grim -g "$(slurp)"`,
`i3-autotiling`, `waybar` e afins funcionem.

Extensões do Orbit vão em **mensagens com números a partir de 1000** e eventos
próprios (`layout`, `tag`, `animation`, `remote`), que clientes do sway ignoram.

Configuração: a sintaxe de **comandos** é a do sway (por isso atalhos e regras
migram quase literais); o **arquivo** é YAML. Um importador
`orbitctl import-sway ~/.config/sway/config` gera o `core.yaml` inicial a partir
da sua config (atalhos, gaps, `for_window`, `workspace … gaps`, `output`).

## 6. Arquitetura de crates

Tudo no workspace do repositório `orbit`, junto com os apps.

| Crate | Papel |
|---|---|
| `orbit-core` | Binário: event loop (calloop), backends (winit aninhado, DRM/libinput via libseat, headless), estado de clientes, renderização |
| `orbit-layout` | Biblioteca pura, sem Smithay: árvore, BSP, mestre-pilha, monocle, rolagem. Entra uma lista de janelas + área, sai geometria. Testável com `cargo test` sem tela |
| `orbit-ipc` | Protocolo i3/sway (servidor no core, cliente para `orbitctl` e para os apps). Substitui o `sway.rs` duplicado em `orbit-common` e `orbit-panel` |
| `orbit-config` | YAML → configuração tipada, validação com mensagens claras, recarga a quente, importador do sway |
| `orbit-render` | Efeitos: bordas, cantos, sombras, blur (dual kawase), animações com curvas. Isolado para poder ser desligado inteiro |
| `orbit-remote-server` | Acesso remoto: a camada de compartilhamento do orbit-core, processo à parte, cliente privilegiado por protocolo privado (como o greet); clientes em Slint, o `orbit-local-station` entre eles. Trait `Session { frames(); inject(); }`, captura por dmabuf, codificação VA-API, WebRTC (GStreamer `webrtcbin`), cliente web, pareamento. Implementações `WlrootsSession` (testar já sobre o sway) e `CoreSession` (dentro do core) |
| `orbit-greet` | A face do login e do bloqueio (seção 7): GTK4, PAM, destrava o chaveiro. Processo à parte do core, cliente privilegiado dele |
| `orbitctl` | Linha de comando (`swaymsg` do Orbit) |

Princípios: o **layout não sabe que existe Wayland** (testes rápidos e
determinísticos); o **IPC não sabe que existe renderização** e é só uma visão do
estado, como os protocolos padrão; os **efeitos são
opcionais em tempo de execução** (um `decoration: off` deixa o core no custo do
sway).

## 7. Orbit-Greet: o login é uma camada do core

O login não é um programa que roda antes da sessão e some: é o **estado
travado do próprio orbit-core**. O core sobe já travado; o `orbit-greet` é a
face desse estado; autenticar **destrava** a camada e revela a sessão que já
estava carregada atrás. A mesma camada é a tela de bloqueio (Super+L, ociosidade,
suspender) — login e bloqueio são uma coisa só, com a mesma aparência.

### 7.1 Como funciona

Decidido com o Juliano em 2026-10-21. Todo mundo faz boot → greeter → sessão →
interface; no Orbit a interface **já está carregada** quando o greet aparece, e
entrar é só abrir os olhos.

1. **Boot:** autologin no tty1 (getty, sem greetd desde 2026-10-21) inicia
   `orbit-core --locked`. Painel, dock e wallpaper sobem atrás da camada; o
   autostart XDG (apps) espera o primeiro destravar. Se o core cair, o getty
   refaz o autologin e o core volta **travado** — nunca cai numa sessão aberta.
2. **Travado desde o quadro zero:** antes de aceitar qualquer cliente, o core já
   está no estado `Locked`. Nesse estado ele **não desenha nenhuma superfície de
   cliente nem entrega entrada a elas**: a tela é **preta**, só com o greet por
   cima. Quem garante o bloqueio é o core, não o greet.
3. **Abrir e fechar os olhos:** a transição é do core. Ao destravar, o preto se
   dissolve (opacidade 1 → 0) **junto** com o blur da sessão (raio máximo → 0),
   na mesma curva, ~400 ms: a sessão aparece desfocada e entra em foco. Ao
   travar, o inverso: a sessão desfoca e escurece até o preto. Fora da
   transição não há blur nem quadro desenhado (regra 3); travado, a sessão
   **nunca** aparece, nem desfocada. A entrada para os clientes é cortada no
   primeiro quadro do fechar e só volta no fim do abrir.
4. **O greet é um cliente privilegiado:** o core lança o `orbit-greet` num socket
   Wayland privado; só as superfícies desse cliente aparecem no estado travado,
   em tela cheia, com o teclado exclusivo. Ele fica num processo à parte para o
   código de autenticação (PAM) não morar dentro do compositor. Na transição o
   greet some/aparece com o mesmo fade.
5. **Se o greet cair, a sessão continua travada:** o core mostra o preto e
   relança o greet (a mesma regra do `ext-session-lock`). Crash do greet nunca
   destrava.
6. **Autenticação:** o greet fala PAM pelo serviço `orbit-greet` (`pam_unix`, que
   checa a senha do próprio usuário via `unix_chkpwd`, como o swaylock). Tentativas
   erradas esperam cada vez mais. Com a senha certa, o greet **destrava o
   chaveiro** (gnome-keyring): com autologin, o PAM do login não tem senha para
   isso.
7. **Destravar:** o greet avisa o core por um pedido do protocolo privado; o core
   roda o autostart XDG (só no primeiro destravar) e abre os olhos.
8. **Bloqueio durante o uso:** o mesmo estado `Locked` (Super+L, ociosidade,
   antes de suspender), com as janelas existindo atrás; fecha os olhos.
   **Regra que não se quebra:** a ociosidade **jamais** trava a tela enquanto
   houver qualquer janela em tela cheia — navegador, vídeo, jogo, o que for —
   em qualquer monitor; nem a ociosidade desliga a tela (DPMS) nesse caso.
   Só o usuário trava, pelas teclas (Super+L) ou por `orbitctl lock`. Somam-se
   a isso os pedidos explícitos (`idle-inhibit-v1`, o inibidor do portal/D-Bus
   que players e navegadores usam).
9. **Vários monitores:** o greet no monitor com o cursor; nos outros, só o preto.
10. **Usuários:** por agora, um só (o dono do autologin). Depois, **seleção de
    usuário**: escolhido o usuário, a sessão dele já começa a carregar atrás da
    camada enquanto a senha é digitada, e o login fica instantâneo. Isso pede
    um processo do sistema (fora da sessão de um usuário) que abra sessões
    logind por usuário — fica para depois do caminho de um usuário estar pronto.

### 7.2 O que o greet mostra

Relógio e data no estilo do painel (mesmo formato "ter 20 out"), foto e nome do
usuário, campo de senha, estado da bateria e da rede, teclado (us alt-intl, com
troca de layout), botões de desligar/reiniciar/suspender (os mesmos do
`confirm::choose` do painel) e o tema do Orbit. Configuração em
`~/.config/orbit/greet.yaml`.

### 7.3 Passos

| # | Passo | Fase |
|---|---|---|
| 1 | Estado `Locked` no core: com ele ligado, a tela é preta, nada de cliente é desenhado nem recebe entrada; `--locked`; teste headless provando que uma janela aberta não aparece no quadro capturado | 1 |
| 2 | Abrir e fechar os olhos: fade do preto + blur (dual kawase, shader GLES próprio) da sessão numa textura fora da tela, só durante a transição — **feito** 2026-10-21 (`eyes.rs`, estados `Closing`/`Opening` em `lock.rs`; 400 ms, curva cúbica; a meio caminho volta de onde está; antes de suspender trava sem transição) | 1 |
| 3 | Socket privado para cliente privilegiado e o protocolo `orbit-greet-v1` (pedir destravar; o core põe a janela do greet no monitor ativo) — **feito** 2026-10-21 | 1 |
| 4 | `ext-session-lock-v1` no core (para lockers de terceiros) — **feito** 2026-10-21 (`session_lock.rs`): o locker pega o mesmo bloqueio do Super+L (fecha os olhos) com as superfícies dele no lugar do greet, uma por monitor, o teclado na do cursor; `locked` só com o preto em todas as telas; o `unlock_and_destroy` dele abre os olhos e o greet não destrava enquanto ele segura. Já travado (pelo Orbit ou por outro locker), um novo recebe `finished`. Locker que morre deixa a sessão travada e o greet assume. Testado aninhado com `orbit-protocols/examples/session-lock` (os quatro casos) | 1 |
| 5 | `orbit-greet` mínimo: campo de senha + PAM, rodando sobre o core aninhado; relançado se cair (teste: matar o greet não destrava) — **feito** 2026-10-21 | 1 |
| 6 | Boot do hp com `--locked` + autostart adiado até o primeiro destravar (`login:` no `core.yaml`) — **feito** 2026-10-21 | 2 |
| 7 | Destravar o gnome-keyring com a senha — **feito** 2026-10-21: `auth optional pam_gnome_keyring.so` no serviço `orbit-greet`, com o greet pondo `XDG_RUNTIME_DIR` no ambiente do PAM (é onde o módulo acha o socket de controle do daemon, ativado pelo socket do systemd do usuário) | 2 |
| 7a | Agente polkit próprio, `orbit-polkit` (no lugar do `lxpolkit`): camada overlay com o teclado, o cartão no visual do greet; a senha conferida pelo `polkit-agent-helper-1` do próprio polkit — **feito** 2026-10-21 | 2 |
| 7b | Prompter do chaveiro no visual do Orbit, no lugar do `gcr-prompter` (o pedido de senha quando um app abre um chaveiro travado ou cria um novo). O chaveiro em si continua o gnome-keyring (ver Riscos, "Chaveiro próprio") — **feito** 2026-10-22: crate `orbit-prompter`, que fala o protocolo do `gcr-prompter` (`org.gnome.keyring.SystemPrompter`: BeginPrompting/PerformPrompt/StopPrompting, PromptReady/PromptDone) e a troca `sx-aes-1` (DH 1536, HKDF-SHA256, AES-128-CBC; conferida contra a `libgcr-base-3` real); cartão do `orbit-polkit`, senha nova pedida duas vezes, a escolha "destravar ao entrar". Ativado pelo D-Bus com `data/dbus/org.gnome.keyring.SystemPrompter.service` em `~/.local/share/dbus-1/services/`; sai 30 s depois do último pedido | 4 |
| 8 | Bloqueio por Super+L, ociosidade e antes de suspender — **feito** 2026-10-21: `idle_lock` por máquina no `core.yaml` (`idle.rs`); antes de suspender, um inibidor `delay` do logind, solto quando o quadro preto está em todas as telas, 2 s no máximo (`sleep.rs`) | 2 |
| 8a | **Tela cheia impede o bloqueio por ociosidade** (§7.1 item 8): qualquer janela em tela cheia, em qualquer monitor, zera o contador de ociosidade e impede travar/apagar a tela; `idle-inhibit-v1` e o inibidor do portal também. Super+L e `orbitctl lock` travam sempre. Teste: vídeo em tela cheia passa do tempo de ociosidade sem travar; sair da tela cheia volta a contar — **feito** 2026-10-21 (`fullscreen.rs`, `screensaver.rs`) | 2 |
| 9 | Visual completo (seção 7.2), vários monitores — **feito** 2026-10-21: teclado, rede e bateria no canto (`status.rs` do greet; layouts do próprio keymap, clique troca pelo IPC do core, que o greet agora recebe), suspender/reiniciar/desligar embaixo (`power.rs`, comandos no `greet.yaml`; reiniciar e desligar perguntam no próprio cartão, "Não" focado, Esc volta); o greet segue o cursor entre monitores, os outros ficam pretos | 2 |
| 10 | Seleção de usuário com a sessão carregando atrás da camada — feito em 2026-10-22 (§7.4, `orbit-login`); falta o teste real com dois usuários | 5 |
| 11 | Extras: digital (fprintd), pareamento do `orbit-remote-server` exigindo a sessão destravada | 5 |

**Pronto (login) quando:** o hp liga direto no greet do Orbit; a senha certa
abre os olhos sobre a sessão com o chaveiro destravado e o autostart rodando;
a errada não revela nada; matar o greet (ou o core) não destrava; Super+L,
ociosidade e suspender fecham os olhos na mesma tela.

### 7.4 Passo 10, seleção de usuário

Proposta e código em 2026-10-22 (crate `orbit-login`; o Juliano criou um
segundo usuário, `merilin`, e pediu para seguir com a proposta como está). O passo pede o que nenhum processo de
um usuário pode fazer: abrir a sessão de **outro** usuário e conferir a senha
dele (o `pam_unix` de um usuário só confere a própria senha). O hp tem um
usuário só, então nada disso aparece nele enquanto não houver um segundo.

**Proposta:**

1. **`orbit-login`, serviço do sistema (root, `seat0`)**, no lugar do autologin do
   getty no tty1. Pequeno, no papel do greetd: abre sessões PAM/logind e mais
   nada. No boot, abre a sessão do usuário padrão sem senha (o autologin de
   hoje, serviço PAM `orbit-login-autologin`) e roda `orbit-core --locked` dele
   no VT1. Se o core cair, a sessão é refeita travada, como o getty faz hoje.
2. **O greet lista os usuários** (AccountsService, `org.freedesktop.Accounts`),
   só quando há mais de um. Escolher outro usuário pede ao `orbit-login`, por um
   socket em `/run/orbit/login.sock`, "prepare B".
3. **Pré-carregar:** o `orbit-login` abre a sessão de B (PAM, `pam_systemd`,
   classe `user`) num VT livre, **sem trocar de VT**, e roda o
   `orbit-core --locked` de B lá. A sessão de B carrega atrás enquanto a senha é
   digitada.
4. **A senha de B** vai do greet de A ao `orbit-login`, que confere com o PAM
   (serviço `orbit-greet`, usuário B; o `pam_gnome_keyring` destrava o chaveiro
   de B como no passo 7, com `XDG_RUNTIME_DIR=/run/user/<uid de B>`). Se a senha
   estiver certa: `Session.Activate` do logind (troca para o VT de B) e o core de
   B destrava.
5. **Quem destrava o core de B:** uma mensagem `unlock` no socket do core, aceita
   só de um par com uid 0 (`SO_PEERCRED`), além do greet de hoje. É a única
   porta nova no core.
6. **Voltar:** Super+L em B mostra o greet de B, com A na lista; escolher A
   ativa o VT1, onde o greet de A pede a senha de A.

**Feito assim (2026-10-22):**

- `orbit-login` (root) roda a si mesmo para cada trabalho com PAM —
  `--session USER VT COMMAND` (o líder da sessão no logind, que fecha a sessão
  quando o comando acaba) e `--check USER` (senha pela entrada padrão) —
  porque os módulos do PAM fazem fork, o que um processo com threads não deve.
  A sessão roda `COMMAND` pelo shell de login do usuário (`-l -c`), com a
  entrada fora do tty: o ramo do tty1 do `~/.profile` não dispara.
- O core de uma sessão aberta atrás **espera ser mostrado** (logind,
  `Session.Active`) antes de pegar a tela: a GPU não é dele até lá. O que
  carrega atrás são os serviços do usuário (systemd `--user`, chaveiro,
  Syncthing); o core e o `startup:` sobem na troca, já travados, e o `unlock`
  do `orbit-login` (tentado por até 30 s) abre os olhos.
- O greet mostra os outros abaixo do cartão só com o `orbit-login` no ar, a
  ligação com o core e mais de uma pessoa (uid 1000–59999 com shell, de
  `/etc/passwd`); Esc volta ao próprio usuário.
- O `orbit-login` só atende programas das sessões que ele abriu (uid por
  `SO_PEERCRED`), uma pergunta por vez, e espera 2, 4, 8… até 30 s depois de
  3 senhas erradas para o mesmo usuário.
- Se a sessão padrão morre em menos de 10 s e nada mais está aberto, o
  serviço falha e o `OnFailure=` devolve o tty1 ao getty (login de texto).
- Instalação: `scripts/install-login.sh` (como root) põe `orbit-login`,
  `orbit-core` e `orbit-greet` em `/usr/local/bin` (de root; os outros
  usuários não têm cópia em `~/.local/bin`), o PAM, a unit e
  `/etc/orbit/login.yaml`; `--remove` volta ao autologin do getty.
- Testado sem root: o `unlock` de um usuário comum é recusado (core aninhado),
  `--check` recusa a senha errada, a lista de usuários e o protocolo (testes).
  **Falta o teste real** (root, dois usuários, troca de VT).

**Decisões tomadas** (eram as perguntas abertas da proposta):

- Um serviço root próprio no lugar do autologin do getty: sim.
- Pré-carregar só ao escolher o usuário, não no boot.
- O canal root → core: o `unlock` no IPC, aceito só de uid 0.
- Fazer agora: sim, com o segundo usuário criado no hp.

## 8. Fases

Cada fase só termina com o critério de "pronto" verificado — em teste
automatizado sempre que der.

### Fase 1 — aninhado (janela dentro do sway), com a sessão real já testável

Ponto de partida: o **esqueleto do anvil** (estado, event loop calloop,
backends winit/headless) com a **organização do cosmic-comp** (`state`,
`backend/{kms,winit}`, `shell/{floating,tiling}`, `input`, `wayland/protocols`,
`xwayland`). Os backends, a entrada e o XWayland são portados do cosmic-comp; o
tiling automático dele dá a base do layout `tree`.

O uso é aninhado (backend winit dentro da sessão sway atual), mas como o backend
DRM vem portado junto, **a sessão real já sobe num TTY de teste** desde o fim
desta fase — sem esperar o aninhado ficar completo.

- Janelas `xdg-shell`, foco (clique e `focus_follows_mouse`), teclado com o
  layout us alt-intl e dead keys, mouse, `floating_modifier`.
- Layout `tree` (split h/v, tabbed, stacked), flutuantes, tela cheia, scratchpad.
- Workspaces numerados, regras `for_window`, modos, gaps e bordas.
- Layer-shell: painel e dock do Orbit sobem dentro dele.
- XWayland.
- IPC compatível (seção 5) + `orbitctl`; importador da config do sway.
- Backend DRM/KMS + libinput via libseat portado do cosmic-comp, entrada `orbit`
  no greetd marcada como experimental.
- Estado `Locked`, a transição de abrir/fechar os olhos, cliente privilegiado,
  `ext-session-lock` e o `orbit-greet` mínimo (passos 1–5 da seção 7.3).
- **Pronto quando:** dentro da janela aninhada, painel, dock, menu, blink,
  orbit-files/view/play/edit/read funcionam como no sway; o `swaymsg -t get_tree`
  responde; a suíte de testes headless passa (abrir, fechar, dividir, mover para
  workspace, flutuar, scratchpad, regras); e a sessão `orbit` sobe num TTY do hp
  com painel e um terminal, mesmo que ainda sem monitor externo.

**Andamento da Fase 1**

- **2026-10-21 — esqueleto aninhado** (`crates/orbit-core`). Decidido com o
  Juliano: porta-se com origem (seção 9; README ajustado) e começa-se pelo
  esqueleto aninhado. Smithay fixado no commit `24647fc` (git), o mesmo em que
  o smallvil/anvil foram lidos. Base no **smallvil** (MIT) + layer shell,
  decoração, seleções e dmabuf do **anvil**; cada arquivo diz de onde veio.
  - Feito: janelas `xdg-shell` lado a lado em colunas iguais (`tile.rs`, com
    teste; a árvore do i3 vem no `orbit-layout`), bordas desenhadas pelo core
    (`#4682b4`/`#4c566a`, 2 px, gaps 6 — da config do sway), decoração no
    servidor, popups com grab (menus do painel), **layer shell** (o painel do
    Orbit sobe e reserva a borda), teclado `us alt-intl` com teclas mortas,
    foco por clique e `focus_follows_mouse`, atalhos Mod+Return / Mod+Q (fecha) / Mod+Shift+Q (mata, `kill -9`) /
    Mod+Shift+E / Mod+setas (Shift move), clipboard (`wl-data-device`,
    `primary-selection`, `wlr-data-control`), `xdg-output`, viewporter,
    `core.yaml` com padrões (vazio = sessão usável), desenho só quando algo
    muda (§1.3).
  - Testado no sway aninhado: painel + orbit-calc + foot + kitty; acento morto
    (`'a` → á), clique chega ao app, Super+Return abre e Super+Shift+Q fecha com
    o foco indo ao vizinho.
  - **Armadilhas:** (1) o smallvil só faz `flush_clients` dentro do desenho, que
    lá é contínuo; com desenho sob demanda os clientes ficavam sem resposta do
    registry — o flush vai a cada volta do loop. (2) Smithay nesse commit
    anuncia **linux-dmabuf v6** sempre que há feedback, e o **GTK 4.18 do Debian
    cai** (SIGSEGV ao abrir o display: v6 não manda `main_device`). Contorno:
    dmabuf v3 + wl_drm do EGL (`use_system_lib`). Voltar ao v4+ quando o GTK
    aceitar (o cosmic-comp usa o mesmo v6). (3) `EGL BAD_SURFACE` logo após
    redimensionar a janela aninhada: só um quadro inteiro, inofensivo. (4) No
    teste, a 1ª tecla de cada execução do `vptr` se perde (troca de keymap do
    teclado virtual), não é do core.
- **2026-10-21 — IPC, áreas de trabalho e `orbitctl`.**
  - Crate `orbit-ipc` (protocolo i3 + cliente): o `orbit-common::sway` virou
    reexportação dele e procura `ORBIT_SOCK` → `SWAYSOCK` → `I3SOCK` (painel e
    blink sem mudança). Crate `orbitctl` (o `swaymsg` do Orbit: `-t`, `-r`,
    `-m`).
  - No core: socket `$XDG_RUNTIME_DIR/orbit-ipc.<uid>.<pid>.sock` exportado nas
    três variáveis; `GET_WORKSPACES/OUTPUTS/TREE/INPUTS/VERSION/SEATS`,
    `SUBSCRIBE` com eventos `workspace` (init/focus/empty), `window`
    (new/close/focus/title/move) e `input`; `RUN_COMMAND` com `;`/`,`,
    critérios `con_id`/`app_id`/`title`/`workspace` (regex) e os comandos do
    dia 1 da seção 5 (workspace number/next/prev/back_and_forth, focus [dir |
    output], kill, move to workspace/scratchpad/left/right, scratchpad show,
    floating disable, xkb_switch_layout, exec, reload, exit). O que ainda não
    existe (split, layout, floating enable, fullscreen, resize, gaps, border)
    responde `success: false` com o motivo.
  - Áreas de trabalho numeradas (uma por vez, Mod+1…0 / Mod+Shift+1…0) e
    scratchpad ("minimizar" do painel); ids de janela estáveis (`con_id`).
  - Testado: painel com `workspaces`, `windows` e `keyboard` dentro do core
    (clique na área 2 troca; minimizar/restaurar), `swaymsg` do sistema contra o
    core, eventos na ordem do sway.
  - Ainda não: árvore/tabbed/flutuantes, regras `for_window`,
    estado `Locked` / greet, testes headless automáticos; migrar painel/blink
    para os protocolos padrão (§5).
- **2026-10-21 — backend DRM/KMS** (`udev.rs`, portado do anvil, MIT): sessão
  libseat (logind), GPU única com `GlesRenderer` sobre GBM, um `DrmOutput` por
  conector (nome `eDP-1`/`HDMI-A-4`, modo preferido, lado a lado), libinput
  (movimento relativo, touchpad no padrão do libinput como no sway), troca de VT
  (Ctrl+Alt+F1…F12, pausa/retoma DRM e libinput), hotplug por udev, ponteiro
  desenhado pelo core (`cursor.rs`: tema `Breeze_Light` 24 ou a imagem do app),
  `render.rs` comum aos dois backends. Desenho sob demanda: um pedido desenha na
  próxima volta ociosa, a não ser que um quadro espere o vblank; o vblank só
  redesenha se algo mudou. Escolha automática: aninhado com
  `WAYLAND_DISPLAY`/`DISPLAY`, tela sem eles (`--nested`/`--drm` forçam). Log
  em `~/.local/state/orbit/core/core.log` no modo tela. Saída de emergência
  Ctrl+Alt+Backspace. Instalado em `~/.local/bin` para o 1º teste num TTY
  (não testável de dentro do sway, que é o dono da GPU).
- **2026-10-21 — estado `Locked`** (§7.3 passo 1, `lock.rs`). `--locked` trava
  antes de o loop aceitar clientes; `lock` no IPC trava a sessão em uso (não
  há `unlock` no IPC: destravar é só do greet). Travado: tela preta, só o
  ponteiro do tema; teclado e ponteiro saem de todo cliente; atalhos
  desligados, menos Ctrl+Alt+Fn e Ctrl+Alt+Backspace; clique e roda não chegam
  a ninguém; foco pedido por janela nova ou menu só é anotado para o
  destravar; commits de cliente não pedem quadro nem recebem frame callback
  (regra 3). Testado aninhado (`--locked -- foot` + `orbitctl lock`).
  - **Falta antes de travar no boot:** o `~/.shell/raiz/profile` roda o core
    sem `exec` — sair (ou o core cair) deixa um shell logado no tty1, o que
    fura o lock. No passo 6 vira `exec orbit-core --locked`.
  - Ainda não: teste headless com captura de quadro (não há backend headless).
- **2026-10-21 — o greet como cliente privilegiado** (§7.3 passo 3,
  `greet.rs`, crate `orbit-protocols`). O core lança o greet (`greet:` no
  `core.yaml`, padrão `orbit-greet`, por `sh -c`) numa ponta de um par de
  sockets, entregue como `WAYLAND_SOCKET`: ser o greet é ter sido lançado
  assim, nunca algo que o cliente declara. Só essa conexão vê o global
  `orbit_greet_v1` (`protocols/orbit-greet-v1.xml`): pedido `unlock`, eventos
  `locked`/`unlocked` (no bind e a cada mudança). Não há "monitor ativo" no
  protocolo: o greet abre um `xdg_toplevel` comum, que **não entra no
  tiling**; travado, ele ocupa o monitor do ponteiro (fullscreen, segue o
  ponteiro entre monitores), recebe teclado, cliques e frame callbacks;
  destravado, fica escondido e o processo pode continuar vivo. A cada
  `lock` o core lança um greet se não houver. Greet que morre travado é
  relançado: na hora se viveu ≥ 5 s, senão após 2, 4, 8… até 30 s; a tela
  segue preta. Testado aninhado com `examples/greet-unlock` (greet de teste
  que destrava N s depois de cada `locked`): `--locked`, `orbitctl lock`,
  matar o greet travado, greet inexistente, e um cliente comum não achando
  o global; com `foot` como greet: configure no tamanho da tela com
  fullscreen, teclado nele e não na sessão.
- **2026-10-21 — `orbit-greet` mínimo** (§7.3 passo 5, crate `orbit-greet`).
  GTK4, `NON_UNIQUE` (um greet relançado nunca entrega a vez a um que está
  saindo). Tela preta com relógio e data no formato do painel, foto
  (`~/.face` ou a do AccountsService), nome (GECOS) e campo de senha. O
  protocolo vai na própria conexão Wayland do GDK, numa fila à parte
  (`wayland-backend` `client_system`, `from_foreign_display`), lida pelo fd
  no loop do GLib. PAM por FFI própria ligada em `libpam.so.0` (sem pacote
  -dev), serviço `orbit-greet` (`data/pam/orbit-greet`: `common-auth` +
  `common-account`; sem o arquivo o PAM usa `other`, que dá no mesmo no
  Debian), em thread; a senha é apagada da memória depois. Duas senhas
  erradas sem espera, depois 2, 4, 8… até 30 s com contagem (além dos ~2 s
  do próprio `pam_unix`). A cada `locked` o campo zera. Fora do core (sem o
  global) é uma prévia: a senha certa só fecha. Entrou no `build-debs.sh`
  e no `orbit-desktop`. Testado: senha errada recusada pelo PAM (teste
  automático); aninhado, o greet recebe `locked` pelo protocolo. Falta o
  Juliano testar digitando a senha certa.
- **2026-10-21 — vários monitores, arrastar, cantos.** Pedidos do Juliano.
  - `output.rs`: monitores sem posição em `outputs:` (`core.yaml`) empilham
    **para baixo**, o do notebook (`eDP`/`LVDS`/`DSI`) em cima, cada um
    centrado no mais largo — o arranjo da mesa do hp. `focus output
    <nome|up|down|left|right>`.
  - Áreas de trabalho por monitor, como no sway: cada monitor mostra a sua;
    a focada é a da janela com o teclado. **Janela nova abre no monitor do
    ponteiro.** Área que não existe nasce no monitor focado; monitor que sai
    deixa as áreas num que ficou. IPC: `output`/`visible` por área, uma
    entrada por monitor em `GET_OUTPUTS` e na árvore.
  - `grab.rs` (do smallvil): **Mod + botão esquerdo arrasta** a janela (e a
    barra de título do próprio app, `move_request`). Solta sobre outra
    janela, as duas trocam de lugar; sobre a parte vazia de outro monitor,
    vai para a área que ele mostra.
  - `rounded.rs`: **cantos arredondados** (`corner_radius`, padrão 10): cada
    superfície da janela passa por um shader de textura que apaga o que fica
    fora do retângulo arredondado (1 px de antialias), achando a posição do
    pixel pela inversa da matriz de textura do Smithay (método do niri); a
    borda virou um anel desenhado por shader de pixel na mesma curva. Menus
    não são cortados. Cada janela é desenhada em ordem (menus, conteúdo,
    borda), então a arrastada fica inteira por cima.
  - Ainda não: redimensionar com Mod + botão direito (não faz sentido nas
    colunas; vem com as flutuantes), monitor girado (cantos supõem
    `Transform::Normal` da saída — o do buffer é tratado).
- **2026-10-21 — XWayland integrado** (`xwayland.rs`, `focus.rs`, portados
  do anvil, MIT). Motivo: o Bambu Studio (wxWidgets) força X11 e saía com
  255 sem servidor X.
  - O core sobe o `Xwayland` (rootless) logo após o backend e é o gerenciador
    de janelas dele (`X11Wm` do Smithay). `DISPLAY` é conhecido já no spawn
    (o core segura o socket), então todo programa iniciado pelo core o
    recebe, inclusive os do `startup:`.
  - Janela X normal entra nas colunas como uma Wayland (`add_window` /
    `remove_window`, comuns às duas); o tamanho que ela pede é ignorado e o
    ladrilho reenviado. Menus e tooltips (override-redirect) vão onde pedem,
    por cima, sem borda nem cantos, e nunca pegam foco nem arraste.
  - Foco: o teclado do seat passou de `WlSurface` para `KeyboardFocus`
    (superfície Wayland ou `X11Surface`), para o X receber o foco de entrada
    (`SetInputFocus`/`WM_TAKE_FOCUS`) mesmo antes da janela ter superfície.
  - Clipboard e seleção primária X ↔ Wayland; fechar (`WM_DELETE_WINDOW`),
    matar pelo `_NET_WM_PID` (nunca o XWayland), arrastar pela barra de
    título; IPC como o sway (`shell: xwayland`, `window_properties`).
  - Testado aninhado: xterm e Bambu Studio lado a lado e focados, clipboard
    nos dois sentidos, `kill` pelo IPC fecha e reorganiza.
  - Ainda não: diálogos X entram nas colunas (vêm com as flutuantes);
    redimensionar/maximizar/tela cheia pedidos pela janela X; escala do
    XWayland em monitor HiDPI.

### Fase 2 — sessão real de uso diário

O backend DRM já existe desde a Fase 1; esta fase é deixá-lo completo para o dia
a dia, portando do cosmic-comp o que falta:

- Múltiplos monitores (eDP-1 + HDMI-A-4 4K), hotplug, escala fracionária,
  `wlr-output-management`.
- Portal (`xdg-desktop-portal-wlr` + gtk), autostart XDG, idle/DPMS.
- Login e bloqueio pelo `orbit-greet` (passos 6–9 da seção 7.3), no lugar do
  console de texto no boot e de bloqueio nenhum (o hp hoje não trava).
- VRR e tearing para jogos.
- **Pronto quando:** um dia inteiro de uso no hp sem voltar ao sway; suspender e
  voltar; conectar/desconectar o monitor externo com janelas abertas; vídeo 4K no
  orbit-play sem quadros perdidos; compartilhamento de tela no navegador; e o
  critério de login da seção 7.3. A partir daqui conta o prazo de duas semanas da regra 1.2.

### Fase 3 — os layouts dos outros

- `bsp` (bspwm), `master` (dwm), `monocle`, `scroll` (niri), layouts por
  workspace e `layout next` (xmonad), scratchpads nomeados
  (Hyprland), snap de flutuantes, marcas e preseleção.
- **Pronto quando:** cada layout tem testes em `orbit-layout` cobrindo inserir,
  remover, mover e redimensionar, e você usou cada um por pelo menos um dia.

### Fase 4 — acabamento

- Animações com curvas (abrir, fechar, mover, trocar workspace), cantos, sombras,
  blur, opacidade por regra, gestos com acompanhamento, overview.
- Desligamento automático na bateria (`effects: { on_battery: off }`).
- Transição animada do greet ao destravar (passo 9 da seção 7.3).
- **Pronto quando:** consumo parado igual ao da Fase 2 (medido com `powertop`);
  animação a 60 fps no iGPU do 7730U com o monitor 4K.

### Fase 5 — o que nenhum outro tem

- `orbit-remote-server` + `orbit-local-station`: acesso remoto nativo
  (decidido em 2026-10-22, detalhes no `docs/pending.md` §9). O servidor é a
  **camada de compartilhamento** do orbit-core, não um segundo Orbit:
  processo à parte, cliente privilegiado do core por protocolo privado; o
  core entrega os quadros (dmabuf, sem cópia) e recebe a entrada remota
  (libei). Mídia no GStreamer (`webrtcbin`, codificadores VA-API, Opus)
  sobre WebRTC, pareamento por token/QR, TLS, acessível de fora pelo túnel
  WireGuard casa↔VPS + TURN. **Rust + Slint** na interface do servidor e em
  todos os clientes: página web (WASM), o `orbit-local-station` (backend
  LinuxKMS do Slint, sem compositor; puxa o greeter e a sessão direto do
  servidor) e, bem depois, Android e iOS; os apps do desktop continuam GTK4.
  Primeiro o espelho das telas, depois uma saída virtual na resolução do
  cliente com as telas locais travadas. **Pronto (fase 1) quando:** latência
  ≤ 20 ms na rede local a 60 fps, medida com um relógio na tela do servidor
  fotografado ao lado da do cliente. Cobaias: o think e a VM Julas. A PoC pode começar **antes**, sobre o sway, com
  `WlrootsSession`.
- Wallpaper Jarvis: camada `background` que reage ao áudio (PipeWire) e sobe a
  `overlay` quando o orbit-aigent escuta; FPS limitado e pausa quando coberto.
- orbit-aigent integrado: atalho do core ativa, comandos viram IPC.
- Configurações de monitor próprias; backend de portal próprio; digital no
  greet (passo 10 da seção 7.3).
- `orbit-keys`: o app de senhas do Orbit, no lugar do Seahorse — ver, buscar,
  copiar e editar o que está no chaveiro, pelo Secret Service (funciona com o
  gnome-keyring hoje e com o oo7 se ele vier).
- **Pronto quando:** abrir a sessão do hp no navegador do celular fora de casa,
  com mouse e teclado, sem instalar nada; e "abrir o arquivos na pasta
  Downloads" por voz funcionar.

### Em paralelo, desde já

- Rodar o core no **think** também (AMD RX 6400), para pegar diferença de GPU
  cedo.
- `orbit-ipc` substitui os dois `sway.rs` atuais já na Fase 1, com os apps
  passando a usar `ORBIT_SOCK` → `SWAYSOCK` como fallback.
- Painel, dock e blink migram para os protocolos padrão (seção 5), um módulo por
  vez, testados no core e, onde der, no sway. O sway 1.10.1 do Debian 13 tem
  `ext-foreign-toplevel-list` mas **não** `ext-workspace` (conferido em
  2026-10-20), então no sway o módulo de áreas mantém o IPC como alternativa.
  Quando todos migrarem, o IPC fica só para scripts e terceiros.
- `orbit-remote-server` estuda o `libei.rs` do cosmic-comp: injeção de mouse e teclado
  pelo padrão libei, o mesmo que o portal de área de trabalho remota usa.

## 9. Referências e como portar

**Decidido (2026-10-20): nada de fork.** O orbit-core começa do zero e
**porta** para dentro do projeto os conceitos e trechos de código dos outros:
entende-se como o projeto de origem resolve o problema, reescreve-se no estilo e
na arquitetura do Orbit (seção 6) e testa-se aqui. Nenhuma dependência de crate
dos outros compositores; só do Smithay.

| Projeto | Linguagem / licença | O que se estuda e porta |
|---|---|---|
| **cosmic-comp** (System76) | Rust, Smithay / GPL-3.0-only | Referência principal: backend DRM e multi-monitor, hotplug, escala fracionária, libseat, XWayland, tiling automático, foco, protocolos de layer-shell e screencopy |
| **niri** | Rust, Smithay / GPL-3.0-or-later | Layout `scroll` (Fase 3), animações com curvas e molas, overview, gestos com acompanhamento, testes de layout sem tela, qualidade do renderizador GLES |
| **anvil** (exemplo do Smithay) | Rust / MIT | Esqueleto mínimo: estado, event loop calloop, backends winit/DRM/headless. Ponto de partida da Fase 1 |
| **Smithay** | Rust / MIT | Dependência (versão exata fixada), não referência |
| **sway** + **wlroots** | C / MIT | IPC e sintaxe de comandos/critérios (seção 5), `GET_TREE`, regras, scratchpad, modos |
| **i3** | C / BSD-3-Clause | Árvore de contêineres: split, tabbed, stacked, `focus parent`, movimentação na árvore |
| **bspwm** | C / BSD-2-Clause | BSP, preseleção, receptáculos, controle 100% externo |
| **xmonad** | Haskell / BSD-3-Clause | Layouts combináveis e modificadores (conceito; o código não porta) |
| **dwm** | C / MIT | Mestre-pilha (`nmaster`, `mfact`), tags, monocle |
| **spectrwm** | C / ISC | Padrões sem configuração, workspaces por monitor |
| **awesome** | C + Lua / GPL-2.0 | Regras e layouts programáveis (só conceito) |
| **Hyprland** | C++ / BSD-3-Clause | Curvas de animação, blur (dual kawase), cantos, sombras, workspaces especiais, gestos |

Regras de porte:

1. **Licença confere antes de copiar.** MIT, BSD, ISC e GPL-3.0-or-later entram
   sem problema. Código do **cosmic-comp (GPL-3.0-only)** pode ser portado, mas o
   arquivo que o recebe passa a ser GPL-3.0-only — anotar no cabeçalho. Haskell,
   Lua e o que for só conceito não têm essa questão. Confirmar a licença no
   repositório de origem no momento do porte.
2. **Todo trecho portado registra a origem** no comentário do módulo: projeto,
   arquivo e commit (ex.: `// Portado de niri src/layout/scrolling.rs @ abc1234`),
   mantendo o aviso de copyright que a licença exigir.
3. **Porta-se o comportamento, não a estrutura.** O código vem para a arquitetura
   do Orbit (`orbit-layout` sem Wayland, `orbit-ipc` sem renderização); não se
   copia módulo inteiro com as dependências internas do projeto de origem.
4. **Teste junto com o porte.** Cada recurso portado chega com o teste headless
   ou de `orbit-layout` que prova que ele se comporta como no original.
5. Clones de referência em `~/Vault/GIT/ref/` (cosmic-comp, niri, smithay com
   anvil, sway, bspwm, dwm), só para leitura, fora do workspace do Orbit.

## 10. Pendências e riscos

| Item | Risco / decisão em aberto |
|---|---|
| Wayfire no think | Fora de pauta por ora: o think e a VM Julas ficam como cobaias do `orbit-local-station` + `orbit-remote-server` (pending §9) |
| Smithay e a API instável | Smithay muda API entre versões; fixar versão exata e atualizar de propósito, como foi com o glycin. O cosmic-comp faz o mesmo (Smithay 0.7 com `[patch]` num commit do git): ao portar um trecho, usar o mesmo commit de Smithay que ele usava naquele ponto, para o código vir funcionando |
| XWayland | Conferido em 2026-10-20: o **cosmic-comp usa o XWayland integrado do Smithay** (`XWayland::spawn` + `X11Wm`, em `src/xwayland.rs`); o **niri usa o `xwayland-satellite`** (processo à parte). Integrado = foco, empilhamento e regras das janelas X sob controle do core, e código pronto para portar; satellite = menos código, mas janelas X viram clientes Wayland comuns e perdem detalhes (posição de menus, ícones na bandeja X). **Proposta: integrado, portado do cosmic-comp**; o satellite fica como plano B se o porte travar a Fase 1 |
| Porte do cosmic-comp | Código grande e amarrado a `cosmic-protocols`, `cosmic-config` e libcosmic: portar só o comportamento, trocando as dependências COSMIC pelas nossas (`orbit-config`, protocolos padrão); arquivos que receberem código dele viram GPL-3.0-only (seção 9) |
| Renderização | GLES2 do Smithay basta para Fases 1–3; blur e animações podem pedir shaders próprios — medir na iGPU antes de prometer |
| Compatibilidade do IPC | Clientes de terceiros (waybar, autotiling) podem usar campos que o Orbit não usa; a meta é o que o Orbit usa + o que aparecer nos testes, não 100% do sway |
| Segurança do remoto | Expor a sessão é o recurso mais perigoso do projeto; desligado por padrão, só via túnel, pareamento explícito, auditoria antes de abrir para fora |
| Tamanho do esforço | Fase 1 sozinha é o maior bloco de código do Orbit até agora; cada fase vira commits pequenos, nunca um despejo único |
| Login automático | O autologin do tty1 abre a sessão do usuário **antes** da senha: serviços de usuário do systemd (Syncthing, PipeWire) já sobem no boot, embora nada apareça na tela até destravar. O `$HOME` não é criptografado, então a senha protege a tela, não os dados em disco. **Aceito em 2026-10-21**: é o preço de "a sessão já estava carregada atrás" (§7.1) |
| Chaveiro no destravar | **Resolvido em 2026-10-21** (§7.3 passo 7): `pam_gnome_keyring` no serviço PAM do `orbit-greet` destrava o chaveiro `login` com a mesma senha |
| Chaveiro próprio | **Decidido em 2026-10-21: não escrever um.** O gnome-keyring não amarra o Orbit ao GNOME: é um daemon do padrão Secret Service (`org.freedesktop.secrets`), que é o que os apps usam, e se troca sem mexer neles. Um chaveiro novo é cripto que guarda todas as senhas, com o formato do `login.keyring` a ler e a API inteira a manter. Se um dia trocar, o candidato é o **oo7** (Rust, do pessoal do GNOME, lê o formato do gnome-keyring), quando estiver maduro no Debian. O Orbit desenha o que se vê: o prompter (§7.3 passo 7b) e um app de senhas (Fase 5) |
| Tags (dwm) | Decidido em 2026-10-22: fora; workspaces (as áreas) são o único modelo. Talvez numa major version depois da versão final |
