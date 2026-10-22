# Orbit — português do Brasil.

## Clock

clock-tooltip-year = semana { $week } · dia { $day } do ano

## Session

session-poweroff = desligar
session-reboot = reiniciar
session-logout = encerrar sessão (logoff)
session-poweroff-question = Desligar o computador?
session-reboot-question = Reiniciar o computador?
session-logout-question = Encerrar a sessão gráfica?
session-menu = sessão
session-yes = Sim
session-no = Não
session-notification-title = Sessão
session-before-failed = A preparação para sair não terminou.
    Sair mesmo assim?
session-leave-anyway = Sair mesmo assim
session-cancel = Cancelar
session-choose = O que deseja fazer?

## Menu de aplicativos

menu-label = Aplicativos
menu-sublabel = Buscar e abrir
menu-command = Executar comando
menu-search = Buscar aplicativos…
menu-power = Desligar, reiniciar ou encerrar a sessão
menu-run = Nenhum aplicativo encontrado. Enter executa “{ $command }”.
menu-category-all = Todos
menu-category-utilities = Acessórios
menu-category-development = Desenvolvimento
menu-category-education = Educação e ciência
menu-category-games = Jogos
menu-category-graphics = Gráficos
menu-category-internet = Internet
menu-category-multimedia = Multimídia
menu-category-office = Escritório
menu-category-settings = Configurações
menu-category-system = Sistema
menu-category-other = Outros

## Window list

windows-show = Mostrar (estava minimizada)
windows-minimize = Minimizar
windows-maximize = Maximizar
windows-restore = Restaurar
windows-area = Área { $area }
windows-close = Fechar
windows-place-area = área { $area }
windows-place-minimized = minimizada

## Workspaces

workspaces-tooltip = área { $area } — em todas as telas (Super+{ $area })

## Volume and brightness

volume-tooltip = volume: { $percent }%{ $muted ->
        [yes] {" "}(mudo)
       *[no] {""}
    } — clique no ícone abre o mixer, botão direito silencia
brightness-tooltip = brilho: { $percent }%

## Network

network-none = sem rede
network-none-tooltip = nenhuma conexão ativa
network-no-address = sem endereço
network-kind-wifi = Wi-Fi
network-kind-wired = cabo
network-kind-vpn = VPN
network-menu-networking = Rede ativada
network-menu-wifi = Wi-Fi ativado
network-menu-edit = Editar conexões…

## Keyboard

keyboard-next = { $name } — clique para o próximo:

## Battery

battery-none = sem bateria
battery-uptime = ligado há { $uptime }
battery-tooltip = bateria { $percent }%, { $state }{ $remaining }
battery-remaining = {" "}— { $time } restantes
battery-charging = carregando
battery-discharging = descarregando
battery-full = carregada
battery-pending = aguardando carga

## Weather

weather-tooltip = { $description } em { $place }
    mínima { $min }°C · máxima { $max }°C · sol { $sunrise }–{ $sunset }
    sensação { $feels }°C · umidade { $humidity }%
    vento { $wind } km/h { $direction } · pressão { $pressure } hPa
    nuvens { $clouds }% · índice UV { $uv }
    lua: { $moon } ({ $lit }% iluminada)
    { $season }
forecast-tooltip = { $weekday } { $date } — { $description }, mínima { $min }°C, máxima { $max }°C

# Rosa dos ventos de 16 pontos, do norte em sentido horário: leste = L, oeste = O.
weather-compass = N NNL NL LNL L LSL SL SSL S SSO SO OSO O ONO NO NNO

weather-code-0 = Céu limpo
weather-code-1 = Predomínio de sol
weather-code-2 = Parcialmente nublado
weather-code-3 = Nublado
weather-code-45 = Nevoeiro
weather-code-48 = Nevoeiro com geada
weather-code-51 = Garoa fraca
weather-code-53 = Garoa
weather-code-55 = Garoa forte
weather-code-56 = Garoa congelante
weather-code-61 = Chuva fraca
weather-code-63 = Chuva
weather-code-65 = Chuva forte
weather-code-66 = Chuva congelante
weather-code-71 = Neve fraca
weather-code-73 = Neve
weather-code-75 = Neve forte
weather-code-80 = Pancadas de chuva
weather-code-81 = Pancadas fortes
weather-code-82 = Pancadas violentas
weather-code-85 = Pancadas de neve
weather-code-95 = Trovoadas
weather-code-96 = Trovoadas com granizo
weather-code-unknown = —

moon-new = lua nova
moon-waxing-crescent = crescente
moon-first-quarter = quarto crescente
moon-waxing-gibbous = crescente gibosa
moon-full = lua cheia
moon-waning-gibbous = minguante gibosa
moon-last-quarter = quarto minguante
moon-waning-crescent = minguante

# The calendar's title: "Primavera 2026".
calendar-title = { $season } { $year }
season-winter = inverno
season-spring = primavera
season-summer = verão
season-autumn = outono

## Dock

dock-open = Abrir
dock-recent = Recentes
dock-edit = Editar dock
dock-edit-done = Concluir edição
dock-add-launcher = Adicionar atalho…
dock-add-appimage = Adicionar AppImage…
dock-add-separator = Adicionar separador
dock-search = Buscar aplicativo

## Calculadora

calc-title = Calculadora
calc-profile-standard = Padrão
calc-profile-scientific = Científica
calc-profile-financial = Financeira
# O visor da calculadora financeira quando uma operação falha (os números de erro da HP 12c).
calc-hp-error = Erro { $code }
calc-angle = Unidade dos ângulos: graus (DEG) ou radianos (RAD). Clique para trocar.
calc-profile-subnet = Sub-rede
calc-decimal-separator = ,
calc-group-separator = .
calc-error-syntax = Expressão incompleta ou inválida
calc-error-division-by-zero = Não é possível dividir por zero
calc-error-domain = Fora do domínio da função
calc-error-overflow = Número grande demais
calc-subnet-prefix = Comprimento do prefixo (bits da máscara)
calc-subnet-hint = Digite um endereço, como 10.0.0.199/24
calc-subnet-bad-address = Não é um endereço IPv4
calc-subnet-bad-mask = Prefixo ou máscara inválidos
calc-subnet-address = Endereço
calc-subnet-mask = Máscara
calc-subnet-wildcard = Curinga (wildcard)
calc-subnet-network = Rede
calc-subnet-broadcast = Broadcast
calc-subnet-first = Primeiro host
calc-subnet-last = Último host
calc-subnet-hosts = Hosts utilizáveis
calc-subnet-size = Endereços
calc-subnet-class = Classe
calc-subnet-kind = Tipo
calc-subnet-binary-address = Endereço (binário)
calc-subnet-binary-mask = Máscara (binário)
calc-subnet-hex-address = Endereço (hexa)
calc-subnet-hex-mask = Máscara (hexa)
calc-subnet-none = nenhum (/31 e /32)
calc-kind-private = privado (RFC 1918)
calc-kind-shared = compartilhado, CGNAT (RFC 6598)
calc-kind-loopback = loopback
calc-kind-link-local = link-local
calc-kind-multicast = multicast
calc-kind-reserved = reservado
calc-kind-broadcast = broadcast
calc-kind-public = público

## Área de transferência

clip-title = Área de transferência
clip-search = Buscar no histórico
clip-empty = Nada copiado ainda
clip-no-match = Nada encontrado
clip-clear = Limpar histórico
clip-count = { $count ->
        [one] 1 item
       *[other] { $count } itens
    }
clip-image = imagem { $width } × { $height }
clip-image-unreadable = imagem (ilegível)
clip-ago-now = agora
clip-ago-minutes = há { $n } min
clip-ago-hours = há { $n } h
clip-ago-days = há { $n } d

## Captura de tela

blink-title = Captura de tela
blink-mode-screen = Tela
blink-mode-window = Janela
blink-mode-area = Área
blink-delay = Atraso (segundos)
blink-cursor = Incluir o ponteiro
blink-take = Capturar
blink-saved = Tela capturada
blink-saved-in = { $size } · na área de transferência
blink-copy = Copiar
blink-copied = Copiado
blink-save-as = Salvar como…
blink-failed = A captura falhou
blink-no-window = Nenhuma janela em foco para capturar.
blink-file-prefix = captura

## Visualizador de imagens

view-title = Orbit-View
view-empty = Nenhuma imagem aberta.
    Arraste uma imagem para cá ou escolha uma.
view-open = Abrir…
view-filter-images = Imagens
view-failed = Não foi possível abrir { $name }.
    { $reason }
view-unknown-format = Formato de imagem não reconhecido.
view-previous = Anterior
view-next = Próxima
view-zoom-out = Diminuir
view-zoom-in = Aumentar
view-fit = Ajustar à janela
view-actual-size = Tamanho real
view-turn-left = Girar para a esquerda
view-turn-right = Girar para a direita
view-mirror-horizontal = Espelhar na horizontal
view-mirror-vertical = Espelhar na vertical
view-save = Gravar a rotação no arquivo
view-save-unsupported = Este formato não pode ser gravado; só JPEG e PNG
view-saved = Gravado
view-save-failed = Não foi possível gravar: { $reason }
view-copy = Copiar a imagem
view-copied = Imagem copiada
view-info = Informações
view-slideshow = Apresentação
view-fullscreen = Tela cheia
view-trash = Mover para a lixeira
view-trashed = { $name } foi para a lixeira
view-trash-failed = Não foi possível mover para a lixeira: { $reason }
view-info-name = Nome
view-info-folder = Pasta
view-info-format = Formato
view-info-dimensions = Dimensões
view-info-file-size = Tamanho do arquivo
view-info-modified = Modificado em
view-info-taken = Fotografado em
view-info-camera = Câmera
view-info-exposure = Exposição
view-info-software = Programa
view-info-place = Local
view-info-coordinate = { $degrees }°{ $minutes }′{ $seconds }″ { $side }
view-info-north = N
view-info-south = S
view-info-east = L
view-info-west = O

## Leitor de documentos

read-title = Orbit-Read
read-empty = Nenhum documento aberto.
    Arraste um PDF, Markdown ou texto para cá, ou escolha um.
read-open = Abrir…
read-open-button = Abrir…
read-filter = Documentos (PDF, Markdown, texto)
read-failed = Não foi possível abrir { $name }.
    { $reason }
read-encrypted = { $name } está protegido por senha.
read-wrong-password = Senha incorreta para { $name }.
read-password = Senha
read-sidebar = Índice e páginas
read-page = Ir para a página
read-of-pages = de { $total }
read-zoom-out = Diminuir
read-zoom-in = Aumentar
read-fit = Ajustar à largura
read-search = Buscar no documento
read-search-placeholder = Buscar…
read-search-previous = Anterior
read-search-next = Próximo
read-search-none = Nada encontrado
read-search-running = Buscando…
read-search-count = { $total } encontrados
read-search-position = { $position } de { $total }
read-search-position-running = { $position } de { $total }…
read-night = Modo noturno
read-print = Imprimir
read-source = Ver o texto-fonte
read-outline = Índice
read-pages = Páginas

## Reprodutor de vídeo e áudio

play-title = Orbit-Play
play-empty = Nada tocando.
    Arraste um vídeo, uma música ou uma animação Flash para cá, ou escolha um arquivo.
play-open = Abrir…
play-filter = Vídeo, áudio e Flash
play-failed = Não foi possível tocar { $name }.
    { $reason }
play-no-ruffle = Para animações Flash é preciso o Ruffle (ruffle.rs), em ~/.local/bin ou no PATH.
play-play = Tocar
play-pause = Pausar
play-previous = Anterior na pasta
play-next = Próximo na pasta
play-frame-back = Quadro anterior
play-frame-forward = Próximo quadro
play-mute = Sem som
play-volume = Volume
play-speed = Velocidade
play-audio-track = Faixa de áudio
play-subtitles = Legendas
play-subtitles-off = Desligadas
play-subtitles-load = Carregar legenda…
play-track = Faixa { $n }
play-mono = mono
play-stereo = estéreo
play-channels = { $n } canais
play-fullscreen = Tela cheia
play-leave-fullscreen = Sair da tela cheia
play-key-space = Espaço

## Editor de texto

edit-title = Orbit-Edit
edit-untitled = Sem título
edit-untitled-file = sem-titulo.txt
edit-new = Novo
edit-open = Abrir…
edit-open-folder = Abrir pasta…
edit-save = Salvar
edit-save-as = Salvar como…
edit-sidebar = Pasta
edit-no-folder = Nenhuma pasta aberta
edit-quick-open = Ir para arquivo…
edit-close-tab = Fechar
edit-close-question = Salvar as alterações em { $name }?
edit-close-detail = Se não salvar, as alterações se perdem.
edit-cancel = Cancelar
edit-discard = Não salvar
edit-open-failed = Não foi possível abrir { $name }: { $reason }
edit-save-failed = Não foi possível salvar: { $reason }
edit-find = Buscar
edit-find-previous = Anterior
edit-find-next = Próximo
edit-find-case = Diferenciar maiúsculas
edit-find-words = Palavras inteiras
edit-find-regex = Expressão regular
edit-find-close = Fechar a busca
edit-find-none = Nada encontrado
edit-find-count = { $total } encontrados
edit-find-position = { $position } de { $total }
edit-replace-with = Substituir por
edit-replace = Substituir
edit-replace-all = Substituir tudo
edit-replaced = { $n } substituídos
edit-goto = Ir para a linha
edit-goto-placeholder = Linha
edit-status-position = Ln { $line }, Col { $column }
edit-status-spaces = Espaços: { $n }
edit-status-tabs = Tabulação: { $n }
edit-indent-spaces = Indentar com espaços
edit-indent-width = Largura
edit-plain-text = Texto puro

## Gerenciador de arquivos

files-title = Orbit-Files
files-back = Voltar
files-forward = Avançar
files-up = Pasta acima
files-filter = Filtrar
files-hidden = Arquivos ocultos
files-split = Dois painéis
files-view-list = Lista
files-view-grid = Ícones
files-view-columns = Colunas
files-column-name = Nome
files-column-size = Tamanho
files-column-modified = Modificado
files-column-kind = Tipo
files-status-loading = Lendo… { $n } itens
files-status-items = { $n } itens
files-status-selected = { $n } de { $total } selecionados ({ $size })
files-cannot-read = Não foi possível ler a pasta: { $reason }
files-preview-empty = Pasta vazia
files-preview-link = atalho
files-places = Lugares
files-home = Pasta pessoal
files-trash = Lixeira
files-computer = Computador
files-bookmarks = Favoritos
files-devices = Dispositivos
files-network = Rede
files-connect = Conectar a servidor…
files-eject = Ejetar
files-open = Abrir
files-open-with = Abrir com
files-open-tab = Abrir em nova aba
files-cut = Recortar
files-copy = Copiar
files-paste = Colar
files-copy-path = Copiar o caminho
files-rename-menu = Renomear…
files-trash-menu = Mover para a lixeira
files-delete-menu = Apagar definitivamente…
files-new-folder = Nova pasta
files-new-folder-name = Nova pasta
files-compress-menu = Compactar…
files-compress = Compactar
files-extract = Extrair aqui
files-archive = arquivo
files-cancel = Cancelar
files-delete = Apagar
files-delete-question = Apagar definitivamente { $n } itens?
files-delete-detail = Eles não vão para a lixeira e não poderão ser recuperados.
files-job-copy = Copiando { $n } itens
files-job-move = Movendo { $n } itens
files-job-trash = Movendo { $n } itens para a lixeira
files-job-delete = Apagando { $n } itens
files-job-compress = Compactando { $name }
files-job-extract = Extraindo { $name }
files-job-stop = Parar
files-job-errors = Alguns itens não puderam ser processados
files-into-itself = { $name } não pode ir para dentro de si mesmo
files-conflict = { $name } já existe aqui
files-conflict-detail = O que fazer com ele?
files-conflict-skip = Pular
files-conflict-skip-all = Pular todos
files-conflict-keep = Manter os dois
files-conflict-replace = Substituir
files-conflict-replace-all = Substituir todos
files-rename = Renomear
files-rename-many = Renomear { $n } itens
files-rename-replace = Buscar e substituir
files-rename-number = Numerar
files-rename-insert = Inserir texto
files-rename-case = Maiúsculas e minúsculas
files-rename-find = Buscar
files-rename-with = Substituir por
files-rename-pattern = Modelo: Viagem ### (* = nome atual)
files-rename-before = Antes do nome
files-rename-after = Depois do nome
files-rename-regex = Expressão regular
files-rename-match-case = Diferenciar maiúsculas
files-rename-apply = Renomear
files-rename-clash = Há nomes repetidos ou inválidos
files-rename-bad-regex = Expressão inválida: { $reason }
files-rename-exists = já existe
files-case-lower = minúsculas
files-case-upper = MAIÚSCULAS
files-case-title = Primeira Maiúscula
files-action-terminal = Abrir terminal aqui

# Orbit-Pathfinder
pathfinder-title = Orbit-Pathfinder
pathfinder-back = Voltar
pathfinder-forward = Avançar
pathfinder-reload = Recarregar
pathfinder-hibernate = Hibernar
pathfinder-open-external = Abrir no navegador externo
pathfinder-add = Adicionar serviço
pathfinder-menu = Configuração
pathfinder-edit-config = Editar configuração
pathfinder-reload-config = Recarregar configuração
pathfinder-name = Nome
pathfinder-url = Endereço
pathfinder-group = Grupo
pathfinder-always-active = Sempre ativo (recebe notificações)
pathfinder-media = Câmera, microfone e chamadas
pathfinder-add-button = Adicionar
pathfinder-mute = Silenciar
pathfinder-unmute = Ativar som
pathfinder-clear = Limpar dados (sair da conta)
pathfinder-clear-confirm = Apagar cookies, logins e cache de { $name }?
pathfinder-remove = Remover
pathfinder-remove-confirm = Remover { $name } da barra? Os dados ficam no disco.
pathfinder-cancel = Cancelar
pathfinder-empty = Nenhum serviço ainda. Use + na barra para adicionar.
pathfinder-group-n = Grupo { $n } ({ $first }…)
pathfinder-new-group = Novo grupo
pathfinder-profile = Perfil
pathfinder-profile-hint = opcional: mesmo nome = mesmo login (ex.: google)
pathfinder-clear-confirm-shared = Apagar cookies, logins e cache de { $name }? O login é compartilhado: { $others } também saem da conta.
pathfinder-fill = Preencher login do Bitwarden (Ctrl+Shift+L)
pathfinder-bw-missing = O Bitwarden CLI (bw) não está instalado.
pathfinder-bw-login = Entre no Bitwarden uma vez no terminal: bw login
pathfinder-bw-master = Senha mestra do Bitwarden
pathfinder-bw-unlock = Desbloquear
pathfinder-bw-wrong = Senha mestra incorreta.
pathfinder-bw-none = Nenhum login do Bitwarden para este site.
pathfinder-import-title = Importar senhas para o Bitwarden…
pathfinder-import-intro = As senhas salvas nestes navegadores vão para o seu cofre do Bitwarden, numa pasta "Importado do …" por navegador. As que o cofre já tem são puladas.
pathfinder-import-source = { $name }: { $count } senhas
pathfinder-import-nothing = Nenhum navegador com senhas salvas foi encontrado.
pathfinder-import-firefox-pw = Senha mestra do Firefox (só se houver)
pathfinder-import-go = Importar
pathfinder-import-working = Importando…
pathfinder-import-done = { $added } senhas adicionadas ao Bitwarden; { $already } já estavam no cofre.
pathfinder-import-failed = A importação falhou: { $error }
pathfinder-import-firefox-needed = Firefox: a senha mestra do Firefox é necessária (ou está errada).
pathfinder-edit = Editar…
pathfinder-save = Salvar
pathfinder-move-up = Mover para cima
pathfinder-move-down = Mover para baixo
pathfinder-icon = Ícone
pathfinder-icon-hint = vazio = o do site; ou endereço de imagem, arquivo
pathfinder-separator-below = Separador abaixo
pathfinder-separator-remove = Remover separador
pathfinder-add-service = Adicionar serviço…
pathfinder-add-separator = Adicionar separador
pathfinder-group-empty = Grupo { $n } (vazio)
pathfinder-browser = Navegador
pathfinder-home = Página inicial (Alt+Home)
pathfinder-address-hint = Digite um endereço ou pesquise (Ctrl+L)
pathfinder-unread = { $count ->
    [one] 1 não lida
   *[other] { $count } não lidas
}
pathfinder-new-tab = Nova aba (Ctrl+T)
pathfinder-close-tab = Fechar aba (Ctrl+W)
pathfinder-cert-title = Certificado não confiável
pathfinder-cert-detail = { $host } é um endereço da sua rede local e apresentou um certificado que não vem de uma autoridade conhecida — comum em Proxmox, roteadores e NAS, que assinam o próprio certificado. Confie só se este for o seu equipamento.
pathfinder-cert-changed = ⚠️ O certificado de { $host } MUDOU desde que você confiou nele. Pode ser uma reinstalação do equipamento — ou alguém se passando por ele.
pathfinder-cert-facts = Emitido para: { $subject }
    Válido até: { $until }
    SHA-256: { $sha256 }
pathfinder-cert-trust = Confiar neste certificado
pathfinder-error-load = Não foi possível abrir a página
pathfinder-error-tls = Certificado inválido
pathfinder-error-tls-detail = O certificado deste site não é válido. O Pathfinder não abre sites públicos com certificado inválido — alguém pode estar se passando por ele.
pathfinder-downloaded = Download concluído: { $name }

## Orbit-Notify, e o sino do painel

notify-title = Notificações
notify-dnd = Não perturbe
notify-clear = Limpar tudo
notify-clear-group = Limpar estas
notify-empty = Nenhuma notificação
notifications-none = nenhuma nova
notifications-new = { $count ->
    [one] 1 nova
   *[other] { $count } novas
}
notifications-dnd = não perturbe
notifications-on = avisos ligados
notifications-off = sem servidor
notifications-tooltip = Clique: central de notificações · Botão direito: não perturbe

## Bluetooth e monitores no painel

bluetooth-name = Bluetooth
bluetooth-none = sem adaptador
bluetooth-off = desligado
bluetooth-on = ligado
bluetooth-connected = conectado
bluetooth-battery = bateria { $percent }%
bluetooth-devices = { $count } conectados
bluetooth-paired-none = Nenhum dispositivo pareado
bluetooth-tooltip = Clique: ligar e conectar · Botão direito: gerenciador
monitors-count = { $count ->
    [one] 1 monitor
   *[other] { $count } monitores
}
monitors-above = { $name } acima
monitors-below = { $name } abaixo
monitors-left = { $name } à esquerda
monitors-right = { $name } à direita
monitors-put-above = Acima do { $main }
monitors-put-below = Abaixo do { $main }
monitors-put-left = À esquerda do { $main }
monitors-put-right = À direita do { $main }
monitors-single = Só um monitor
monitors-editor-hint = Arraste um monitor para o lado de outro
monitors-apply = Aplicar
monitors-tooltip = Clique: posição rápida · Botão direito: editor

## Edição do painel

panel-edit = Editar painel
panel-edit-done = Concluir edição
panel-add = Adicionar módulo
panel-size-mini = Mini
panel-size-normal = Normal
panel-size-full = Completo
panel-move-up = Mover para cima
panel-move-down = Mover para baixo
panel-part-start = No topo
panel-part-center = No meio
panel-part-end = Embaixo
panel-remove = Remover do painel
panel-module-menu = Menu
panel-module-windows = Lista de janelas
panel-module-clock = Relógio
panel-module-calendar = Calendário
panel-module-weather = Clima
panel-module-forecast = Previsão
panel-module-tray = Bandeja do sistema
panel-module-volume = Volume
panel-module-brightness = Brilho
panel-module-network = Rede
panel-module-keyboard = Teclado
panel-module-battery = Bateria
panel-module-notifications = Notificações
panel-module-bluetooth = Bluetooth
panel-module-monitors = Monitores
panel-module-workspaces = Áreas de trabalho
panel-module-session = Sessão
panel-module-spectrum = Espectro de áudio
panel-module-cpu = CPU
panel-module-ram = RAM
panel-module-gpu = GPU
panel-module-lan = Rede (LAN)
panel-module-disk = Disco
panel-filesystem = Sistema de arquivos

# Disk module (orbit-panel)
disk-total = TOTAL
disk-used = USADO
disk-free = LIVRE

# Orbit-Term
term-history = Saídas
term-no-shell = O shell não pôde ser iniciado
term-starting = O shell ainda está iniciando
term-running = Executando { $command }…
term-interactive = { $command } — no terminal
term-exit = saída { $code }
term-stderr = escreveu erros
term-from-history = { $command }  ·  { $time }  ·  { $cwd }

## Login e tela de bloqueio

greet-password = Senha
greet-checking = Verificando…
greet-wrong = Senha incorreta
greet-wait = Tente de novo em { $seconds } s
greet-suspend = suspender

## Pedido de senha do polkit

polkit-title = Autenticação necessária
polkit-authenticate = Autenticar
polkit-cancel = Cancelar
polkit-details = Detalhes
