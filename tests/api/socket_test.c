#include <assert.h>
#include <stdbool.h>
#include <stdint.h>
#include <signal.h>
#include <string.h>
#include "platform.inc"

#define JUSTLOG(...) ((void)0)
#define WARNLOG(...) ((void)0)

static bool control = true;
static int subscriptions, disconnects, requests;
static bool game_control_enabled(void) { return control; }
static void game_control_disconnect(void) { ++disconnects; }
void api_clear_all_subscriptions(void) { subscriptions = 0; }
static uint64_t SDL_GetTicks(void) { return 100; }
static void api_check_var_update(void) {}
#include "client.inc"
#include "send.inc"

static void api_process_buffer(const char *buffer, size_t length)
{
    assert(length == 5 && memcmp(buffer, "state", length) == 0);
    ++requests;
    api_send("ok\n", 3);
}
static void api_process_multipart_json(const char *buffer, size_t length)
{
    api_process_buffer(buffer, length);
}
#include "update.inc"

static struct sockaddr_in address;
static void wait_readable(int fd);

static int connect_client(void)
{
    int peer = socket(AF_INET, SOCK_STREAM, 0);
    assert(peer >= 0);
    struct timeval timeout = {1, 0};
    assert(setsockopt(peer, SOL_SOCKET, SO_RCVTIMEO, &timeout, sizeof(timeout)) == 0);
    assert(connect(peer, (struct sockaddr*)&address, sizeof(address)) == 0);
    wait_readable(api.serverSocket);
    return peer;
}

static void wait_readable(int fd)
{
    fd_set readers;
    FD_ZERO(&readers);
    FD_SET(fd, &readers);
    struct timeval timeout = {1, 0};
    assert(select(fd + 1, &readers, NULL, NULL, &timeout) == 1);
}

static void test_replacement(bool stale)
{
    int previous = connect_client();
    api_update_server();
    assert(api.activeSocket != KFX_INVALID_SOCKET);
    assert(send(previous, "partial", 7, 0) == 7);
    wait_readable(api.activeSocket);
    api_update_server();
    assert(control_buffer_used == 7);
    subscriptions = 1;
    int before = disconnects;
    if (stale) {
        assert(shutdown(previous, SHUT_RDWR) == 0);
        close(previous);
        wait_readable(api.activeSocket);
    }
    int next = connect_client();
    api_update_server();
    assert(disconnects == before + 1);
    assert(subscriptions == 0);
    assert(control_buffer_used == 0);
    assert(send(next, "state\n", 6, 0) == 6);
    wait_readable(api.activeSocket);
    api_update_server();
    char response[3];
    assert(recv(next, response, sizeof(response), MSG_WAITALL) == 3);
    assert(memcmp(response, "ok\n", 3) == 0);
    if (!stale) {
        assert(recv(previous, response, sizeof(response), 0) == 0);
        close(previous);
    }
    api_drop_client();
    close(next);
}

static void test_closed_peer_write(bool enabled)
{
    control = enabled;
    int peer = connect_client();
    api_update_server();
#ifdef SO_NOSIGPIPE
    int protected = 0;
    socklen_t size = sizeof(protected);
    assert(getsockopt(api.activeSocket, SOL_SOCKET, SO_NOSIGPIPE, &protected, &size) == 0);
    assert(protected == 1);
#endif
    assert(shutdown(peer, SHUT_RDWR) == 0);
    close(peer);
    wait_readable(api.activeSocket);
    char byte;
    assert(recv(api.activeSocket, &byte, 1, 0) == 0);
    // TCP can still buffer writes after FIN; force EPIPE without racing the reset.
    assert(shutdown(api.activeSocket, SHUT_WR) == 0);
    subscriptions = 1;
    int before = disconnects;
    api_send("reply\n", 6);
    assert(api.activeSocket == KFX_INVALID_SOCKET);
    assert(subscriptions == 0);
    assert(disconnects == before + (enabled ? 1 : 0));
    api_send("ignored\n", 8);
}

static void test_legacy_rejection(void)
{
    control = false;
    int first = connect_client();
    api_update_server();
    int active = api.activeSocket;
    int second = connect_client();
    api_update_server();
    assert(api.activeSocket == active);
    char byte;
    assert(recv(second, &byte, 1, 0) == 0);
    close(second);
    api_drop_client();
    close(first);
}

int main(void)
{
    assert(signal(SIGPIPE, SIG_DFL) != SIG_ERR);
    api.serverSocket = socket(AF_INET, SOCK_STREAM, 0);
    assert(api.serverSocket >= 0);
    address.sin_family = AF_INET;
    address.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    assert(bind(api.serverSocket, (struct sockaddr*)&address, sizeof(address)) == 0);
    socklen_t size = sizeof(address);
    assert(getsockname(api.serverSocket, (struct sockaddr*)&address, &size) == 0);
    assert(listen(api.serverSocket, 4) == 0);
    assert(fcntl(api.serverSocket, F_SETFL, O_NONBLOCK) == 0);
    test_replacement(false);
    test_replacement(true);
    assert(requests == 2);
    test_closed_peer_write(true);
    test_closed_peer_write(false);
    test_legacy_rejection();
    close(api.serverSocket);
    return 0;
}
