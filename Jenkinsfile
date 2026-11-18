pipeline {
    agent {
        label 'docker'
    }
    stages {
        stage('Wait Docker'){
            agent {
                dockerfile {
                    additionalBuildArgs ''
                    reuseNode true
                }
            }

            steps {
                sh 'echo "ready"'
            }

        }
        stage('Targets') {
            parallel{
                stage('Static Analysis'){
                    agent {
                        dockerfile {
                            additionalBuildArgs '--target static-analysis'
                            args '-e HOME=/tmp -e BUILD_CONTEXT=static-analysis'
                            reuseNode true
                        }
                    }
                    stages {
                        stage("Run") {
                            steps {
                                sh 'scripts/static_analysis.sh'
                            }
                        }
                    }
                }
                stage('x86_64 Full'){
                    agent {
                        dockerfile {
                            additionalBuildArgs '--target native'
                            args '-e HOME=/tmp -e BUILD_CONTEXT=native-full'
                            reuseNode true
                        }
                    }
                    stages {
                        stage("Build") {
                            steps {
                                sh '''
                                SCRUTINY_ENABLE_DATALOGGING=1 \
                                SCRUTINY_SUPPORT_64BITS=1 \
                                SCRUTINY_SUPPORT_PROTECTED_REGIONS=1 \
                                scripts/build.sh
                                '''
                            }
                        }
                        stage("Test") {
                            steps {
                                sh '''
                                SCRUTINY_ENABLE_DATALOGGING=1 \
                                SCRUTINY_SUPPORT_64BITS=1 \
                                SCRUTINY_SUPPORT_PROTECTED_REGIONS=1 \
                                scripts/runtests.sh
                                '''
                            }
                        }
                    }
                }
                stage('i686 Full'){
                    agent {
                        dockerfile {
                            additionalBuildArgs '--target native-i686'
                            args '-e HOME=/tmp -e BUILD_CONTEXT=native-i686-full'
                            reuseNode true
                        }
                    }
                    stages {
                        stage("Build") {
                            steps {
                                sh '''
                                SCRUTINY_TARGET=i686-unknown-linux-gnu \
                                SCRUTINY_ENABLE_DATALOGGING=1 \
                                SCRUTINY_SUPPORT_64BITS=1 \
                                SCRUTINY_SUPPORT_PROTECTED_REGIONS=1 \
                                scripts/build.sh
                                '''
                            }
                        }
                        stage("Test") {
                            steps {
                                sh '''
                                SCRUTINY_TARGET=i686-unknown-linux-gnu \
                                SCRUTINY_ENABLE_DATALOGGING=1 \
                                SCRUTINY_SUPPORT_64BITS=1 \
                                SCRUTINY_SUPPORT_PROTECTED_REGIONS=1 \
                                scripts/runtests.sh
                                '''
                            }
                        }
                    }
                }
                stage('thumbv6m-none-eabi'){
                    agent {
                        dockerfile {
                            additionalBuildArgs '--target cross'
                            args '-e HOME=/tmp -e BUILD_CONTEXT=thumbv6m-none-eabi'
                            reuseNode true
                        }
                    }
                    steps {
                        sh '''
                        SCRUTINY_TARGET=thumbv6m-none-eabi \
                        SCRUTINY_ENABLE_DATALOGGING=1 \
                        SCRUTINY_SUPPORT_64BITS=1 \
                        SCRUTINY_SUPPORT_PROTECTED_REGIONS=1 \
                        scripts/build.sh

                        SCRUTINY_TARGET=thumbv6m-none-eabi \
                        SCRUTINY_ENABLE_DATALOGGING=0 \
                        SCRUTINY_SUPPORT_64BITS=0 \
                        SCRUTINY_SUPPORT_PROTECTED_REGIONS=0 \
                        scripts/build.sh
                        '''
                    }
                }
                stage('thumbv7em-none-eabihf'){
                    agent {
                        dockerfile {
                            additionalBuildArgs '--target cross'
                            args '-e HOME=/tmp -e BUILD_CONTEXT=thumbv7em-none-eabihf'
                            reuseNode true
                        }
                    }
                    steps {
                        sh '''
                        SCRUTINY_TARGET=thumbv7em-none-eabihf \
                        SCRUTINY_ENABLE_DATALOGGING=1 \
                        SCRUTINY_SUPPORT_64BITS=1 \
                        SCRUTINY_SUPPORT_PROTECTED_REGIONS=1 \
                        scripts/build.sh

                        SCRUTINY_TARGET=thumbv7em-none-eabihf \
                        SCRUTINY_ENABLE_DATALOGGING=0 \
                        SCRUTINY_SUPPORT_64BITS=0 \
                        SCRUTINY_SUPPORT_PROTECTED_REGIONS=0 \
                        scripts/build.sh
                        '''
                    }
                }
                stage('riscv32imac-unknown-none-elf'){
                    agent {
                        dockerfile {
                            additionalBuildArgs '--target cross'
                            args '-e HOME=/tmp -e BUILD_CONTEXT=riscv32imac-unknown-none-elf'
                            reuseNode true
                        }
                    }
                    steps {
                        sh '''
                        SCRUTINY_TARGET=riscv32imac-unknown-none-elf \
                        SCRUTINY_ENABLE_DATALOGGING=1 \
                        SCRUTINY_SUPPORT_64BITS=1 \
                        SCRUTINY_SUPPORT_PROTECTED_REGIONS=1 \
                        scripts/build.sh

                        SCRUTINY_TARGET=riscv32imac-unknown-none-elf \
                        SCRUTINY_ENABLE_DATALOGGING=0 \
                        SCRUTINY_SUPPORT_64BITS=0 \
                        SCRUTINY_SUPPORT_PROTECTED_REGIONS=0 \
                        scripts/build.sh
                        '''
                    }
                }
                stage('aarch64-unknown-none'){
                    agent {
                        dockerfile {
                            additionalBuildArgs '--target cross'
                            args '-e HOME=/tmp -e BUILD_CONTEXT=aarch64-unknown-none'
                            reuseNode true
                        }
                    }
                    steps {
                        sh '''
                        SCRUTINY_TARGET=aarch64-unknown-none \
                        SCRUTINY_ENABLE_DATALOGGING=1 \
                        SCRUTINY_SUPPORT_64BITS=1 \
                        SCRUTINY_SUPPORT_PROTECTED_REGIONS=1 \
                        scripts/build.sh

                        SCRUTINY_TARGET=aarch64-unknown-none \
                        SCRUTINY_ENABLE_DATALOGGING=0 \
                        SCRUTINY_SUPPORT_64BITS=0 \
                        SCRUTINY_SUPPORT_PROTECTED_REGIONS=0 \
                        scripts/build.sh
                        '''
                    }
                }
                stage('powerpc64-unknown-linux-gnu'){
                    agent {
                        dockerfile {
                            additionalBuildArgs '--target cross'
                            args '-e HOME=/tmp -e BUILD_CONTEXT=powerpc64-unknown-linux-gnu'
                            reuseNode true
                        }
                    }
                    steps {
                        sh '''
                        SCRUTINY_TARGET=powerpc64-unknown-linux-gnu \
                        SCRUTINY_ENABLE_DATALOGGING=1 \
                        SCRUTINY_SUPPORT_64BITS=1 \
                        SCRUTINY_SUPPORT_PROTECTED_REGIONS=1 \
                        scripts/build.sh

                        SCRUTINY_TARGET=powerpc64-unknown-linux-gnu \
                        SCRUTINY_ENABLE_DATALOGGING=0 \
                        SCRUTINY_SUPPORT_64BITS=0 \
                        SCRUTINY_SUPPORT_PROTECTED_REGIONS=0 \
                        scripts/build.sh
                        '''
                    }
                }
            }
        }
        stage('Configs 1'){
            parallel{
                stage('64bits - No Datalogging'){
                    agent {
                        dockerfile {
                            additionalBuildArgs '--target native'
                            args '-e HOME=/tmp -e BUILD_CONTEXT=native-64bits-nodl'
                            reuseNode true
                        }
                    }
                    stages {
                        stage("Build") {
                            steps {
                                sh '''
                                SCRUTINY_ENABLE_DATALOGGING=0 \
                                SCRUTINY_SUPPORT_64BITS=1 \
                                SCRUTINY_SUPPORT_PROTECTED_REGIONS=1 \
                                scripts/build.sh
                                '''
                            }
                        }
                        stage("Test") {
                            steps {
                                sh '''
                                SCRUTINY_ENABLE_DATALOGGING=0 \
                                SCRUTINY_SUPPORT_64BITS=1 \
                                SCRUTINY_SUPPORT_PROTECTED_REGIONS=1 \
                                scripts/runtests.sh
                                '''
                            }
                        }
                    }
                }
                stage('32bits'){
                    agent {
                        dockerfile {
                            additionalBuildArgs '--target native'
                            args '-e HOME=/tmp -e BUILD_CONTEXT=native-32bits'
                            reuseNode true
                        }
                    }
                    stages {
                        stage("Build") {
                            steps {
                                sh '''
                                SCRUTINY_ENABLE_DATALOGGING=1 \
                                SCRUTINY_SUPPORT_64BITS=0 \
                                SCRUTINY_SUPPORT_PROTECTED_REGIONS=1 \
                                scripts/build.sh
                                '''
                            }
                        }
                        stage("Test") {
                            steps {
                                sh '''
                                SCRUTINY_ENABLE_DATALOGGING=1 \
                                SCRUTINY_SUPPORT_64BITS=0 \
                                SCRUTINY_SUPPORT_PROTECTED_REGIONS=1 \
                                scripts/runtests.sh
                                '''
                            }
                        }
                    }
                }
                stage('32bits - No Datalogging'){
                    agent {
                        dockerfile {
                            additionalBuildArgs '--target native'
                            args '-e HOME=/tmp -e BUILD_CONTEXT=native-32bits-nodl'
                            reuseNode true
                        }
                    }
                    stages {
                        stage("Build") {
                            steps {
                                sh '''
                                SCRUTINY_ENABLE_DATALOGGING=0 \
                                SCRUTINY_SUPPORT_64BITS=0 \
                                SCRUTINY_SUPPORT_PROTECTED_REGIONS=1 \
                                scripts/build.sh
                                '''
                            }
                        }
                        stage("Test") {
                            steps {
                                sh '''
                                SCRUTINY_ENABLE_DATALOGGING=0 \
                                SCRUTINY_SUPPORT_64BITS=0 \
                                SCRUTINY_SUPPORT_PROTECTED_REGIONS=1 \
                                scripts/runtests.sh
                                '''
                            }
                        }
                    }
                }
            }
        }
        stage('Configs 2'){
            parallel{
                stage('64bits - No Prot. regions'){
                    agent {
                        dockerfile {
                            additionalBuildArgs '--target native'
                            args '-e HOME=/tmp -e BUILD_CONTEXT=native-64bits-noprot'
                            reuseNode true
                        }
                    }
                    stages {
                        stage("Build") {
                            steps {
                                sh '''
                                SCRUTINY_ENABLE_DATALOGGING=1 \
                                SCRUTINY_SUPPORT_64BITS=1 \
                                SCRUTINY_SUPPORT_PROTECTED_REGIONS=0 \
                                scripts/build.sh
                                '''
                            }
                        }
                        stage("Test") {
                            steps {
                                sh '''
                                SCRUTINY_ENABLE_DATALOGGING=1 \
                                SCRUTINY_SUPPORT_64BITS=1 \
                                SCRUTINY_SUPPORT_PROTECTED_REGIONS=0 \
                                scripts/runtests.sh
                                '''
                            }
                        }
                    }
                }
                stage('64bits - No Datalogging - No Prot. regions'){
                    agent {
                        dockerfile {
                            additionalBuildArgs '--target native'
                            args '-e HOME=/tmp -e BUILD_CONTEXT=native-64bits-nodl-noprot'
                            reuseNode true
                        }
                    }
                    stages {
                        stage("Build") {
                            steps {
                                sh '''
                                SCRUTINY_ENABLE_DATALOGGING=0 \
                                SCRUTINY_SUPPORT_64BITS=1 \
                                SCRUTINY_SUPPORT_PROTECTED_REGIONS=0 \
                                scripts/build.sh
                                '''
                            }
                        }
                        stage("Test") {
                            steps {
                                sh '''
                                SCRUTINY_ENABLE_DATALOGGING=0 \
                                SCRUTINY_SUPPORT_64BITS=1 \
                                SCRUTINY_SUPPORT_PROTECTED_REGIONS=0 \
                                scripts/runtests.sh
                                '''
                            }
                        }
                    }
                }
                stage('32bits - No Prot. regions'){
                    agent {
                        dockerfile {
                            additionalBuildArgs '--target native'
                            args '-e HOME=/tmp -e BUILD_CONTEXT=native-32bits-noprot'
                            reuseNode true
                        }
                    }
                    stages {
                        stage("Build") {
                            steps {
                                sh '''
                                SCRUTINY_ENABLE_DATALOGGING=1 \
                                SCRUTINY_SUPPORT_64BITS=0 \
                                SCRUTINY_SUPPORT_PROTECTED_REGIONS=0 \
                                scripts/build.sh
                                '''
                            }
                        }
                        stage("Test") {
                            steps {
                                sh '''
                                SCRUTINY_ENABLE_DATALOGGING=1 \
                                SCRUTINY_SUPPORT_64BITS=0 \
                                SCRUTINY_SUPPORT_PROTECTED_REGIONS=0 \
                                scripts/runtests.sh
                                '''
                            }
                        }
                    }
                }
                stage('32bits - No Datalogging - No Prot. regions'){
                    agent {
                        dockerfile {
                            additionalBuildArgs '--target native'
                            args '-e HOME=/tmp -e BUILD_CONTEXT=native-32bits-nodl-noprot'
                            reuseNode true
                        }
                    }
                    stages {
                        stage("Build") {
                            steps {
                                sh '''
                                SCRUTINY_ENABLE_DATALOGGING=0 \
                                SCRUTINY_SUPPORT_64BITS=0 \
                                SCRUTINY_SUPPORT_PROTECTED_REGIONS=0 \
                                scripts/build.sh
                                '''
                            }
                        }
                        stage("Test") {
                            steps {
                                sh '''
                                SCRUTINY_ENABLE_DATALOGGING=0 \
                                SCRUTINY_SUPPORT_64BITS=0 \
                                SCRUTINY_SUPPORT_PROTECTED_REGIONS=0 \
                                scripts/runtests.sh
                                '''
                            }
                        }
                    }
                }
            }
        }
    }
    post {
        // Clean after build
        always {
            deleteDir()
        }
    }
}
