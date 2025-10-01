[2025-10-01 13:30:27] [Conn] Sending message of type <class 'pynicotine.slskmessages.FileSearchResponse'> to user insane_in_the_brain2 on new connection
[2025-10-01 13:30:27] [Msg] OUT: <S - GetPeerAddress> {'user': 'insane_in_the_brain2', 'ip_address': None, 'port': None, 'obfuscation_type': None, 'obfuscated_port': None}
[2025-10-01 13:30:27] [Conn] Requesting address for user insane_in_the_brain2
[2025-10-01 13:30:27] [Conn] Attempting direct connection of type P to user insane_in_the_brain2, address ('127.0.0.1', 53507)
[2025-10-01 13:30:27] [Msg] OUT: <S - ConnectToPeer> {'token': 567916, 'user': 'insane_in_the_brain2', 'conn_type': 'P', 'ip_address': None, 'port': None, 'privileged': None, 'obfuscation_type': None, 'obfuscated_port': None}
[2025-10-01 13:30:27] [Conn] Requesting indirect connection to user insane_in_the_brain2 with token 567916
[2025-10-01 13:30:27] [Msg] IN: <S - GetPeerAddress> {'user': 'insane_in_the_brain2', 'ip_address': '127.0.0.1', 'port': 53507, 'obfuscation_type': 1, 'obfuscated_port': 53508}
[2025-10-01 13:30:27] [Conn] Established outgoing connection of type P with user insane_in_the_brain2. List of outgoing messages: [<pynicotine.slskmessages.FileSearchResponse object at 0x10902f340>]
[2025-10-01 13:30:27] [Conn] Sending peer init message of type P to user insane_in_the_brain2
[2025-10-01 13:30:27] [Msg] OUT: <I - PeerInit> {'sock': <socket.socket fd=16, family=2, type=1, proto=0, laddr=('127.0.0.1', 55594), raddr=('127.0.0.1', 53507)>, 'init_user': 'insane_in_the_brain6', 'target_user': 'insane_in_the_brain2', 'conn_type': 'P', 'outgoing_msgs': [<pynicotine.slskmessages.FileSearchResponse object at 0x10902f340>], 'token': 0}
[2025-10-01 13:30:27] [Msg] OUT: <P - FileSearchResponse> {'search_username': 'insane_in_the_brain6', 'token': 1, 'freeulslots': True, 'ulspeed': 0, 'inqueue': 0, 'unknown': 0}
