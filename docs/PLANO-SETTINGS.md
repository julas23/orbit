# Plano do Orbit-Settings

Status: **planejado** — decidido com o usuário em 2026-11-15. Acompanhamento
em `docs/pending.md` §0 (a trilha do Orbit-Settings).

Origem: o Orbit-Settings tem hoje duas páginas (papel de parede e monitores,
mais o menu do desktop). O resto vive em YAML (`core.yaml`, `keys.yaml`,
`panel.yaml`, `dock.yaml`, `notify.yaml`…) ou em ferramentas de fora (o
módulo de rede abre o `nm-connection-editor`). O objetivo: **tudo o que se
configura num desktop configurável pelo Orbit-Settings**, cobrindo o que as
configurações do GNOME 48, do KDE Plasma 6, do XFCE e do Unity oferecem —
menos o que é exclusivo de cada um deles (seção 6).

## 1. Regras

1. **Busca no topo**, que acha qualquer opção de qualquer página (o nome, a
   descrição e palavras-chave traduzidas de cada opção) e abre a página nela.
   Com tantas páginas, é obrigatória.
2. **Páginas próprias e simples para 90% dos casos**, com um **"Avançado…"**
   que abre a ferramenta especialista para o resto: o `nm-connection-editor`
   (802.1X corporativo, rotas), o `system-config-printer` (impressoras
   exóticas), e assim por diante. Reescrever essas ferramentas não compensa.
3. **Cada página fala direto com o serviço padrão do Linux pelo D-Bus**
   (NetworkManager, BlueZ, accountsservice, timedated…), e o polkit pede a
   senha quando precisa. O que não tem serviço com polkit passa pelo
   `orbit-services-helper` (`docs/PLANO-DISTRO.md` §12.2): comandos
   tipados e validados, nunca texto livre num arquivo de root.
4. **Vale na hora:** cada mudança é aplicada ao soltar o controle, sem botão
   "Aplicar". Mudanças que podem deixar a tela ou a rede inutilizáveis
   (resolução, firewall, a rede em uso) voltam sozinhas depois de 15 a 30 s
   sem confirmação, como os monitores já fazem.
5. **O YAML continua sendo a fonte** para o que é do Orbit: a página regrava
   só as chaves que muda (`ConfigDir::save_app_keys`), mantendo comentários
   e chaves desconhecidas, e quem edita o arquivo à mão não perde nada. O
   componente recarrega sozinho (o core pelo `reload`, o painel e o dock
   pelo monitor de arquivos).
6. **Por máquina quando precisa:** o `/home` é compartilhado entre máquinas
   (NFS, virtiofs), então o que depende do hardware (monitores, touchpad,
   energia, áudio) fica sob `HOST/…`, como os monitores já ficam.
7. **Os módulos do painel abrem a página certa** no clique direito (a parte
   "completa" da regra de cliques do painel): o de rede abre Rede e Wi-Fi, o
   de volume abre Som, o de bateria abre Energia, o de Bluetooth abre
   Bluetooth, o de monitores abre Monitores.
8. **As preferências de cada app ficam no próprio app** (Files, Term, Edit,
   View, Read, Play, Pathfinder…), na janela de preferências dele, como o
   GNOME e o KDE fazem. O Orbit-Settings cobre o sistema e o ambiente.
9. **Endereço de cada página:** `orbit-settings <página>[/<seção>]` (como o
   `orbit-settings wallpaper` de hoje), usado pelo painel, pelo Orbit-Run e
   pelos outros apps.

## 2. Estrutura

Uma janela única (já é: chamada de novo, a que está aberta vem para a frente
na página pedida), com a busca no topo, a barra lateral em grupos e a página
à direita. Cada página é um módulo do crate com o seu índice de opções para a
busca.

## 3. Páginas

### 3.1 Sistema e hardware

| Página | O que cobre | Por trás |
|---|---|---|
| **Rede e Wi-Fi** | Wi-Fi (redes ao alcance, senha, esquecer), cabo, VPN (WireGuard, OpenVPN pelos plugins do NetworkManager), proxy, hotspot, modem 4G; IP fixo e DNS por conexão | NetworkManager, ModemManager; "Avançado…" → `nm-connection-editor` |
| **Bluetooth** | ligar e desligar, parear, conectar, esquecer, enviar arquivo, nome visível | BlueZ |
| **Unidades de rede** | mapear SMB, NFS, SFTP e WebDAV: endereço, pasta, montar ao entrar, a senha no chaveiro (Secret Service); para todos os usuários, montagem no boot | GVfs na sessão; `systemd.mount` + `automount` pelo auxiliar para todos |
| **Som** | saída e entrada padrão, volume e mudo, volume por aplicativo, perfis das placas, sons do sistema, cancelamento de eco do microfone | PipeWire (protocolo do PulseAudio, como o painel) |
| **Monitores** *(existe)* | resolução, taxa, escala, rotação; e a **luz noturna** com horário (do pôr ao nascer do sol ou fixo) | orbit-core (`output`, wlr-gamma-control) |
| **Energia** | perfil de energia, apagar a tela, bloquear e suspender depois de X min (na tomada e na bateria), o que a tampa e o botão de energia fazem, limite de carga da bateria quando o hardware deixa | power-profiles-daemon, UPower, logind (pelo auxiliar), orbit-core (`idle`) |
| **Teclado** | layouts e variantes, ordem e troca, repetição (atraso e velocidade), tecla Compose, Num Lock ao entrar, métodos de entrada (fcitx5, ibus) | orbit-core (`keyboard:`), `/etc/default/keyboard` para o greet |
| **Mouse, touchpad e tablet** | velocidade, aceleração, toque para clicar, rolagem natural, desligar ao digitar, canhoto, botões da caneta | orbit-core (`devices.rs`) |
| **Impressoras e scanners** | adicionar (descoberta na rede, IPP Everywhere), padrão, fila, cancelar, página de teste, opções (papel, duplex); scanners encontrados | CUPS pelo `cups-pk-helper`, Avahi, SANE; "Avançado…" → `system-config-printer` |
| **Armazenamento** | uso por disco e pasta, o que fazer ao conectar um pendrive, câmera ou celular (abrir no Files, importar fotos, nada), montagem automática | udisks2 |
| **Dispositivos** | Thunderbolt (autorizar), perfis de cor dos monitores e impressoras, qual GPU usar em notebooks híbridos, rotação automática, atualização de firmware | bolt, colord, switcheroo-control, iio-sensor-proxy, fwupd |

### 3.2 Pessoas e região

| Página | O que cobre | Por trás |
|---|---|---|
| **Usuários e grupos** | criar e remover, administrador ou não, foto, nome, senha, idioma; grupos (criar, quem pertence); digital e rosto (com o orbit-safe e o greet, §3 do `pending.md`); o usuário que abre no boot | accountsservice, fprintd, howdy, `login.yaml`, auxiliar para os grupos |
| **Data e hora** | fuso (com mapa ou busca), hora pela rede, formato de 24 h, relógios do mundo do painel | timedated, `modules/clock.yaml` |
| **Região e idioma** | idioma da interface, formatos (datas, números, moeda, papel), idiomas extras | localed, accountsservice |
| **Privacidade** | serviço de localização, histórico de arquivos recentes (limpar, por quanto tempo), esvaziar lixeira e temporários sozinho, permissões dos apps (câmera, microfone, tela, localização) | geoclue, `recently-used.xbel`, xdg-permission-store |

### 3.3 O Orbit

| Página | O que cobre | Hoje está em |
|---|---|---|
| **Aparência** | claro ou escuro, cor de destaque, tema GTK e Qt, ícones, cursor (e tamanho), fontes (interface, documentos, monoespaçada, tamanho, suavização), escala do texto; papel de parede e efeitos *(existe)* | gsettings (que o portal repassa aos apps), `core.yaml` |
| **Janelas e áreas** | layout padrão e por área, gaps, borda, cores, cantos, foco segue o mouse, a grade das áreas, Alt+Tab e os cards, regras de janela (`rules:`) | `core.yaml` |
| **Atalhos** | cada atalho do `keys.yaml`, mudado capturando a combinação, com aviso de conflito; os atalhos próprios (combinação → comando) | `keys.yaml` |
| **Painel** | borda, tamanho, monitor, os módulos (ligar, ordem, tamanho) e as opções de cada módulo; o modo de edição no lugar continua | `panel.yaml`, `modules/*.yaml` |
| **Dock** | borda, tamanho, monitor, lançadores e grupos, ocultar sozinho | `dock.yaml` |
| **Notificações** | não perturbe (com horário), por app (mostrar, som, guardar no centro), posição, tempo na tela | `notify.yaml` |
| **Tela de bloqueio e login** | bloquear depois de X min, papel de parede do greet, mostrar a lista de usuários, o usuário que abre no boot | `core.yaml`, `login.yaml`, `greet.yaml` |
| **Aplicativos padrão e início** | navegador, e-mail, terminal, arquivos, música, vídeo, imagens, por tipo de arquivo; o que abre ao entrar (com atraso, por máquina) | `mimeapps.list`, `core.yaml` (`terminal:`, `startup:`), autostart do XDG |
| **Serviços** *(planejada)* | a página de `docs/PLANO-DISTRO.md` §12.2 | — |
| **Atualizações** | procurar e instalar, atualizar sozinho (só segurança ou tudo), repositórios, o histórico | PackageKit, unattended-upgrades |
| **Inicialização** | tempo do menu de boot, kernel padrão, os snapshots e "voltar para este", o tema do boot | systemd-boot, snapper (pelo auxiliar) |
| **Acessibilidade** | tamanho do texto, alto contraste, menos animação, tamanho do cursor, lupa, teclado na tela, teclas de aderência e de repetição lenta, teclas do mouse, alerta visual, leitor de tela | orbit-core (quase tudo é trabalho novo no core), Orca |
| **Sobre** | nome da máquina, hardware (CPU, memória, GPU, discos), versão do Orbit e do Debian, o relatório para colar num bug | hostnamed |
| **Editor avançado** | cada YAML de `~/.config/orbit/`, cru, com validação antes de salvar e o erro apontado na linha, como o editor de configurações do XFCE | `~/.config/orbit/` |

## 4. Fases

### Base (junto com a etapa 2 do plano de serviços)

A janela com a busca, a barra lateral, o índice de opções, o endereço de
cada página e a página Serviços, que é a primeira nova.

### Fase A — o uso de todo dia

Rede e Wi-Fi, Bluetooth, Som, Energia, Aparência, Teclado, Mouse e touchpad,
Data e hora, Região e idioma, Usuários e grupos, Painel, Dock, Janelas e
áreas, Atalhos, Notificações, Aplicativos padrão e início, Sobre; e os
módulos do painel abrindo as páginas (regra 7).

**Pronta quando:** no hp e no think, nada do dia a dia precisa mais de YAML à
mão nem de uma ferramenta de fora, e a busca acha cada opção dessas páginas.

### Fase B

Unidades de rede, Impressoras e scanners, Armazenamento, Privacidade,
Atualizações, Tela de bloqueio e login, Inicialização.

**Pronta quando:** uma pasta SMB montada ao entrar, uma impressora da rede
adicionada e uma atualização instalada, tudo pelo Orbit-Settings.

### Fase C

Acessibilidade (com o trabalho no orbit-core que ela pede), Dispositivos,
Editor avançado.

**Pronta quando:** cada opção de acessibilidade funciona na sessão e no
greet, e o editor avançado salva qualquer YAML do Orbit sem perder
comentários.

## 5. Pacotes

O `orbit-settings` passa a depender (ou recomendar) do que as páginas usam e
o Debian não traz por padrão: `accountsservice`, `cups-pk-helper`,
`power-profiles-daemon`, `fprintd` (Recommends), `bolt`, `colord`,
`switcheroo-control`, `iio-sensor-proxy` e `fwupd` (Recommends). As
ferramentas do "Avançado…" (`nm-connection-editor`, `system-config-printer`)
ficam como Recommends.

## 6. Fora do plano

O que é exclusivo de outro ambiente fica de fora (o usuário, 2026-11-15):

- KDE Connect e GSConnect (o celular integrado ao desktop).
- Contas online (GNOME Online Accounts, as contas do KDE).
- Atividades do KDE, provedores de busca do GNOME, indexação de arquivos
  (Baloo, Tracker).
- Efeitos e scripts do KWin, temas do Plasma, o editor de menus do XFCE.

## 7. Riscos

- **Acessibilidade é trabalho no core**, não só uma página: lupa, teclas de
  aderência, teclado na tela e o leitor de tela (AT-SPI) pedem código novo
  no orbit-core e no greet.
- **Quantidade:** são umas trinta páginas. A regra 2 (o "Avançado…") e as
  fases mantêm o tamanho sob controle; cada página nova entra com o seu
  índice para a busca, ou a busca envelhece.
- **YAML editado à mão e pela página ao mesmo tempo:** a página lê o arquivo
  de novo antes de gravar, e regrava só as suas chaves (regra 5).
