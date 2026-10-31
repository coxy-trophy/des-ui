import socket,sys
connection=socket.socket(socket.AF_UNIX);connection.settimeout(60);connection.connect('/tmp/desui-uat-control.sock');connection.sendall((sys.argv[1]+'\n').encode());print(connection.makefile().readline().strip())
