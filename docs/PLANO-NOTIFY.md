# Plano do Orbit-Notify

Status: **rascunho para validação** (2026-10-21), com a primeira versão já
implementada (seção 8).
Origem: pedido de 2026-10-20 — "quando houver notificações nos atalhos do
Pathfinder, enviar para a central de notificações", e a central em si no painel.

## 1. O que é

O **servidor de notificações** do Orbit: o processo que atende
`org.freedesktop.Notifications` no D-Bus de sessão, no lugar do **dunst**.
Faz três coisas:

1. **Balões** — mostra cada notificação no canto da tela por alguns segundos.
2. **Central** — guarda o histórico e o mostra num painel lateral, agrupado por
   aplicativo, com *não perturbe* e *limpar tudo*.
3. **Contador** — diz ao `orbit-panel` quantas notificações não lidas há, para o
   módulo `notifications` mostrar o sino com o número.

## 2. Decisões

| Pergunta | Decisão | Por quê |
|---|---|---|
| O painel **lê** o dunst ou **substitui**? | **Substitui.** O Orbit é o servidor. | Ler o dunst exigiria `dunstctl history` periódico, o que fere a regra 3 do PLANO-PAINEL (nada de processos para coletar dados). Como a bandeja, que já tem watcher próprio. |
| Dentro do `orbit-panel` ou processo à parte? | **Processo à parte: `orbit-notify`.** | Um travamento do painel não pode derrubar as notificações, nem o contrário (mesma regra que separa painel e dock). O painel conversa com ele por D-Bus, por evento. |
| Como o painel sabe do contador? | Interface própria `io.github.julas23.Orbit.Notify`, com o sinal `Changed(u não lidas, b não perturbe)`. | Evento, não consulta. |
| Como abrir a central? | Clicar no sino do painel; rodar `orbit-notify` de novo (atalho do sway); `orbit-notify toggle`. | Igual ao `orbit-clip`: o segundo lançamento mostra ou esconde a janela. |
| Wayland ou X11? | **Só Wayland** (layer-shell), como os demais. | hp e think são Wayland-only. |
| Onde ficam os dados? | Histórico em `~/.local/state/orbit/notify/history.json`; opções em `~/.config/orbit/notify.yaml`. | Mesmo padrão do Pathfinder e do Clip. O histórico é estado, não dado do usuário: não precisa ir para o Syncthing. |

## 3. Comportamento

### Balões

- Canto **superior direito** do monitor configurado, um embaixo do outro, no
  máximo `max_popups` (5) por vez; os demais esperam a vez.
- Ícone (imagem enviada, `image-path`, `app_icon` ou ícone do `.desktop`),
  nome do app, título em negrito e corpo (com o markup simples da especificação:
  `<b> <i> <u> <a>`; o que não for válido vira texto puro).
- Tempo: o que o app pedir (`expire_timeout`); senão, por urgência — baixa 5 s,
  normal 8 s, crítica **fica até ser fechada** (como no dunst de hoje).
- Ponteiro em cima **pausa** o tempo.
- Clique esquerdo: roda a ação `default` (se houver) e fecha; sem ação, abre o
  app que notificou (pelo `desktop-entry`). Clique direito: fecha.
  Botões para as demais ações.
- Notificação com o mesmo `replaces_id` atualiza o balão (e o item da central)
  em vez de criar outro — é o que o Pathfinder faz com "N não lidas".
- Barra de progresso quando vier o hint `value` (volume, brilho).
- Urgência crítica: borda vermelha (`@orbit_alert`).

### Central

- Painel no lado direito da tela, altura toda, 400 px, por cima das janelas.
- Cabeçalho: título, botão *não perturbe*, botão *limpar tudo*.
- Lista agrupada por app (o grupo mais recente em cima), com a hora de cada uma;
  cada grupo pode ser limpo de uma vez, cada item tem o seu ×.
- Clique no item: mesma regra do balão (ação `default` se o app ainda estiver
  ouvindo; senão abre o app). O item sai da central.
- Abrir a central **zera as não lidas**.
- Esc ou clicar de novo no sino fecham.

### Não perturbe

- Balões param (menos os de urgência crítica); tudo continua indo para a central.
- O sino do painel troca de ícone. Lembra o estado entre reinícios.

### Fora do histórico

- Notificações com o hint `transient`, e as dos apps listados em `no_history`
  (hoje no dunst: `volume`, `microfone`), só aparecem como balão.

## 4. Configuração

```yaml
# ~/.config/orbit/notify.yaml — tudo opcional
monitor: 0            # índice ou nome da saída (HDMI-A-1)
width: 360            # largura dos balões
center_width: 400     # largura da central
margin: 10            # distância da borda
max_popups: 5
timeout: { low: 5, normal: 8, critical: 0 }   # segundos; 0 = até fechar
history_limit: 200
no_history: [volume, microfone]
```

```yaml
# ~/.config/orbit/modules/notifications.yaml — o sino do painel
icon_size: 20
```

Ligar o sino = pôr `notifications` numa das listas do `panel.yaml`.

## 5. Interface D-Bus

**`org.freedesktop.Notifications`** em `/org/freedesktop/Notifications`
(especificação 1.2): `Notify`, `CloseNotification`, `GetCapabilities`
(`body`, `body-markup`, `body-hyperlinks`, `actions`, `icon-static`,
`persistence`), `GetServerInformation`; sinais `NotificationClosed`,
`ActionInvoked`, `ActivationToken`.

**`io.github.julas23.Orbit.Notify`** em `/io/github/julas23/Orbit/Notify`
(para o painel e para atalhos):

| Membro | Tipo | Faz |
|---|---|---|
| `Toggle()` | método | abre ou fecha a central |
| `SetDnd(b)` | método | liga/desliga não perturbe |
| `Clear()` | método | limpa a central |
| `Unread` | propriedade `u` | não lidas |
| `Dnd` | propriedade `b` | não perturbe |
| `Changed(u, b)` | sinal | mudou contador ou não perturbe |

Linha de comando: `orbit-notify [--background]`, `orbit-notify toggle|dnd|clear`.

## 6. Troca do dunst

1. Instalar (`scripts/install.sh` passa a incluir o `orbit-notify`).
2. No sway: trocar `exec dunst` por `exec ~/.local/bin/orbit-notify --background`
   e pôr `notifications` no `panel.yaml`. **Quem troca é o Juliano** (mexe no
   `$HOME` ao vivo); dá para testar antes sem mexer em nada: `pkill dunst;
   orbit-notify` — o dunst volta com `dunst &`.
3. Se o dunst estiver rodando, o `orbit-notify` avisa no log e espera o nome
   ficar livre (assume assim que o dunst sair) — nunca derruba o outro.
4. Depois de uma semana sem falta: `apt purge dunst`.

## 7. Riscos e pendências

- **Imagens grandes** no hint `image-data` (fotos de contato): reduzir para 48 px
  na chegada e não gravar no histórico os bytes brutos (só o caminho, se houver).
- **Links no corpo** (`<a href>`): abrir com o navegador padrão (Pathfinder).
- **Sons** (hint `sound-file`): fora desta versão.
- **Ação depois de reiniciar:** o app que mandou pode ter saído; a central então
  só abre o app. Ações de notificações antigas (do histórico salvo) não valem.
- **Monitor seguindo o mouse** (dunst `follow = mouse`): fora desta versão; o
  monitor é fixo pela config.
- **orbit-core:** quando o compositor próprio existir, o *não perturbe*
  pode seguir tela cheia / apresentação.

## 8. Etapas

| # | Etapa | Estado |
|---|---|---|
| 1 | Servidor `org.freedesktop.Notifications` + balões | ✅ 2026-10-21 |
| 2 | Histórico persistente + central | ✅ 2026-10-21 |
| 3 | Interface própria + módulo `notifications` do painel | ✅ 2026-10-21 |
| 4 | Não perturbe, `no_history`, progresso | ✅ 2026-10-21 |
| 5 | Teste lado a lado com o dunst e troca no sway (Juliano) | ✅ 2026-10-21 |
| 6 | Purgar o dunst | ✅ 2026-10-21 (hp) |
