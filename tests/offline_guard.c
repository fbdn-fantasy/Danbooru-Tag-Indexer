// Development QA only: deny and record non-loopback Internet attempts.
#define _GNU_SOURCE
#include <dlfcn.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <netdb.h>
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
static int external(const struct sockaddr *a){
 if(!a)return 0;
 if(a->sa_family==AF_INET)return (ntohl(((const struct sockaddr_in*)a)->sin_addr.s_addr)>>24)!=127;
 if(a->sa_family==AF_INET6)return !IN6_IS_ADDR_LOOPBACK(&((const struct sockaddr_in6*)a)->sin6_addr);
 return 0;
}
static void record(const char *s){const char*p=getenv("TAG_CODEX_NETWORK_LOG");if(p){FILE*f=fopen(p,"a");if(f){fprintf(f,"%s\n",s);fclose(f);}}}
int connect(int fd,const struct sockaddr*a,socklen_t l){if(external(a)){record("external connect blocked");errno=ENETUNREACH;return -1;}return ((int(*)(int,const struct sockaddr*,socklen_t))dlsym(RTLD_NEXT,"connect"))(fd,a,l);}
ssize_t sendto(int fd,const void*b,size_t n,int flags,const struct sockaddr*a,socklen_t l){if(external(a)){record("external sendto blocked");errno=ENETUNREACH;return -1;}return ((ssize_t(*)(int,const void*,size_t,int,const struct sockaddr*,socklen_t))dlsym(RTLD_NEXT,"sendto"))(fd,b,n,flags,a,l);}
int getaddrinfo(const char*n,const char*s,const struct addrinfo*h,struct addrinfo**r){if(n&&strcmp(n,"localhost")&&strcmp(n,"127.0.0.1")&&strcmp(n,"::1")){record(n);return EAI_NONAME;}return ((int(*)(const char*,const char*,const struct addrinfo*,struct addrinfo**))dlsym(RTLD_NEXT,"getaddrinfo"))(n,s,h,r);}
